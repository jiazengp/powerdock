//! Negotiated HID++ feature capabilities, identity and battery precision.
use super::{Device, Observation};
use crate::{
    battery::{self, Kind, Level, Power, Reading, Value},
    identity::Evidence,
};
use hidpp::{
    channel::{ChannelError, HidppChannel},
    device::{Device as ProtocolDevice, DeviceError},
    feature::{
        CreatableFeature,
        battery_status::{
            BatteryStatusFeature, LegacyBatteryCapability, LegacyBatteryInfo, LegacyBatteryStatus,
        },
        device_information::DeviceInformationFeature,
        device_type_and_name::{DeviceType, DeviceTypeAndNameFeature},
        unified_battery::{
            BatteryCapabilities, BatteryInfo, BatteryLevel, BatteryStatus, UnifiedBatteryFeature,
        },
    },
    protocol::v20::Hidpp20Error,
};
use std::sync::Arc;

pub(super) fn channel_error(error: ChannelError) -> String {
    match error {
        ChannelError::Timeout | ChannelError::NoResponse => "timeout: HID++ device did not respond",
        ChannelError::Implementation(_) => "disconnected: HID++ transport failed",
        _ => "invalid_reply: HID++ channel rejected response",
    }
    .into()
}
fn feature_error(error: Hidpp20Error) -> String {
    match error {
        Hidpp20Error::Channel(e) => channel_error(e),
        _ => "invalid_reply: HID++ feature reply".into(),
    }
}

enum BatterySource {
    Unified {
        feature: Arc<UnifiedBatteryFeature>,
        capability: BatteryCapabilities,
    },
    Legacy {
        feature: Arc<BatteryStatusFeature>,
        capability: LegacyBatteryCapability,
    },
}
pub(super) struct CachedDevice {
    info: Option<(Arc<DeviceInformationFeature>, u8)>,
    kind: Kind,
    battery: BatterySource,
}
impl CachedDevice {
    pub(super) async fn initialize(channel: Arc<HidppChannel>, slot: u8) -> Result<Self, String> {
        let mut device = ProtocolDevice::new(channel, slot)
            .await
            .map_err(|e| match e {
                DeviceError::Channel(e) => channel_error(e),
                DeviceError::DeviceNotFound => {
                    "disconnected: HID++ peripheral did not answer".into()
                }
                // Unknown HID++1.0 receiver/register layouts are never guessed.
                _ => "unsupported: HID++1.0 live battery probing awaits exact peripheral evidence"
                    .into(),
            })?;
        let root = device.root();
        let info = root
            .get_feature(DeviceInformationFeature::ID)
            .await
            .map_err(feature_error)?
            .map(|meta| {
                (
                    device.add_feature::<DeviceInformationFeature>(meta.index),
                    meta.version,
                )
            });
        let kind = if let Some(meta) = root
            .get_feature(DeviceTypeAndNameFeature::ID)
            .await
            .map_err(feature_error)?
        {
            let feature = device.add_feature::<DeviceTypeAndNameFeature>(meta.index);
            kind(feature.get_device_type().await.map_err(feature_error)?)
        } else {
            Kind::Other
        };
        let battery = if let Some(meta) = root
            .get_feature(UnifiedBatteryFeature::ID)
            .await
            .map_err(feature_error)?
        {
            let feature = device.add_feature::<UnifiedBatteryFeature>(meta.index);
            let capability = feature
                .get_battery_capabilities()
                .await
                .map_err(feature_error)?;
            BatterySource::Unified {
                feature,
                capability,
            }
        } else if let Some(meta) = root
            .get_feature(BatteryStatusFeature::ID)
            .await
            .map_err(feature_error)?
        {
            let feature = device.add_feature::<BatteryStatusFeature>(meta.index);
            let capability = feature
                .get_battery_capability()
                .await
                .map_err(feature_error)?;
            BatterySource::Legacy {
                feature,
                capability,
            }
        } else {
            return Err("unsupported: HID++ device exposes no accepted battery feature".into());
        };
        Ok(Self {
            info,
            kind,
            battery,
        })
    }
    pub(super) async fn read(&self, device: &Device) -> Result<Observation, String> {
        let identity = if let Some((feature, version)) = &self.info {
            let info = feature.get_device_info().await.map_err(feature_error)?;
            unit_identity(*version, info.unit_id, info.model_id)
                .unwrap_or_else(|| Evidence::Route(device.id.clone()))
        } else {
            Evidence::Route(device.id.clone())
        };
        let reading = match &self.battery {
            BatterySource::Unified {
                feature,
                capability,
            } => map_unified(
                capability,
                feature.get_battery_info().await.map_err(feature_error)?,
                self.kind,
            ),
            BatterySource::Legacy {
                feature,
                capability,
            } => map_legacy(
                capability,
                feature
                    .get_battery_level_status()
                    .await
                    .map_err(feature_error)?,
                self.kind,
            ),
        };
        Ok(Observation { reading, identity })
    }
}
fn kind(value: DeviceType) -> Kind {
    match value {
        DeviceType::Mouse | DeviceType::Trackball | DeviceType::Trackpad => Kind::Mouse,
        DeviceType::Keyboard | DeviceType::Numpad => Kind::Keyboard,
        DeviceType::Headset => Kind::Headset,
        DeviceType::Gamepad | DeviceType::Joystick | DeviceType::SteeringWheel => Kind::Gamepad,
        _ => Kind::Other,
    }
}
fn unit_identity(version: u8, unit: [u8; 4], mut models: [u16; 3]) -> Option<Evidence> {
    if version < 1
        || unit == [0; 4]
        || unit == [255; 4]
        || models.iter().all(|m| matches!(*m, 0 | 0xffff))
    {
        return None;
    }
    models.sort_unstable();
    Some(Evidence::Unit(battery::key(
        "logitech-unit",
        &format!("{unit:02x?}:{models:04x?}"),
    )))
}
fn map_unified(cap: &BatteryCapabilities, info: BatteryInfo, kind: Kind) -> Reading {
    let power = match info.status {
        BatteryStatus::Discharging => Power::Battery,
        BatteryStatus::Charging
        | BatteryStatus::ChargingNearlyFull
        | BatteryStatus::ChargingSlow
            if cap.rechargeable =>
        {
            Power::Charging
        }
        BatteryStatus::Full if cap.rechargeable => Power::Full,
        _ => Power::Unknown,
    };
    let fault = matches!(
        info.status,
        BatteryStatus::InvalidBattery | BatteryStatus::ThermalError | BatteryStatus::ChargingError
    );
    let value = if fault {
        Value::Unknown
    } else if cap.percentage {
        Value::reported(info.charging_percentage)
    } else if cap.reported_levels.contains(&info.level) {
        match info.level {
            BatteryLevel::Critical => Value::Ordinal {
                level: Level::Critical,
            },
            BatteryLevel::Low => Value::Ordinal { level: Level::Low },
            BatteryLevel::Good => Value::Ordinal { level: Level::High },
            BatteryLevel::Full => Value::Ordinal { level: Level::Full },
            _ => Value::Unknown,
        }
    } else {
        Value::Unknown
    };
    Reading::new(value, kind, power)
}
fn map_legacy(cap: &LegacyBatteryCapability, info: LegacyBatteryInfo, kind: Kind) -> Reading {
    let power = match info.status {
        LegacyBatteryStatus::Discharging => Power::Battery,
        LegacyBatteryStatus::Recharging
        | LegacyBatteryStatus::AlmostFull
        | LegacyBatteryStatus::SlowRecharge => Power::Charging,
        LegacyBatteryStatus::Full => Power::Full,
        _ => Power::Unknown,
    };
    // 0x1000's zero is not a level when charging. Full does not fabricate 100%.
    let value = if info.status != LegacyBatteryStatus::Discharging
        || info.discharge_level > 100
        || !(2..=100).contains(&cap.number_of_levels)
    {
        Value::Unknown
    } else if cap.reports_mileage() {
        Value::reported(info.discharge_level)
    } else {
        Value::Ordinal {
            level: match info.discharge_level {
                0..=10 => Level::Critical,
                11..=29 => Level::Low,
                30..=80 => Level::High,
                _ => Level::Full,
            },
        }
    };
    Reading::new(value, kind, power)
}

#[cfg(test)]
mod tests;
