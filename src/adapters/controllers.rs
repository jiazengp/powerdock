//! Battery-only controller APIs. Automatic discovery is intentionally disabled:
//! WGI's app-local ID and XInput's user slot do not prove a PnP/BT association.
//! https://learn.microsoft.com/en-us/uwp/api/windows.gaming.input.rawgamecontroller.nonroamableid
use crate::{
    battery::{self, Kind, Level, Power, Reading, Source, Value},
    identity::Evidence,
};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use windows::{
    Gaming::Input::RawGameController,
    System::Power::BatteryStatus,
    Win32::UI::Input::XboxController::{
        BATTERY_DEVTYPE_GAMEPAD, BATTERY_LEVEL, BATTERY_LEVEL_EMPTY, BATTERY_LEVEL_FULL,
        BATTERY_LEVEL_LOW, BATTERY_LEVEL_MEDIUM, BATTERY_TYPE, BATTERY_TYPE_ALKALINE,
        BATTERY_TYPE_DISCONNECTED, BATTERY_TYPE_NIMH, BATTERY_TYPE_WIRED,
        XINPUT_BATTERY_INFORMATION, XInputGetBatteryInformation, XUSER_MAX_COUNT,
    },
};
use windows_sys::Win32::Foundation::{ERROR_DEVICE_NOT_CONNECTED, ERROR_SUCCESS};

pub const DISABLED_REASON: &str =
    "控制器电量实验路径未启用：WGI/XInput 与蓝牙物理设备尚无可靠关联证据";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
#[value(rename_all = "lower")]
pub enum Backend {
    Wgi,
    XInput,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Endpoint {
    Wgi(RawGameController),
    XInput(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub container: Option<String>,
    pub kind: Kind,
    endpoint: Endpoint,
}

/// No duplicate-prone controller fallback enters normal Bluetooth/PnP discovery.
pub fn enumerate() -> Result<Vec<Device>, String> {
    Ok(Vec::new())
}

/// Explicit, isolated hardware-validation entry point. Select one backend per run
/// and do not combine these route-only rows with Bluetooth/PnP observations.
/// IDs are leases for this enumeration, never persisted physical identities.
/// The caller initializes its COM apartment before invoking WinRT APIs.
pub fn enumerate_experimental(backend: Backend) -> Result<Vec<Device>, String> {
    static NEXT_LEASE: AtomicU64 = AtomicU64::new(1);
    let lease = NEXT_LEASE.fetch_add(1, Ordering::Relaxed);
    match backend {
        Backend::Wgi => {
            let controllers = RawGameController::RawGameControllers()
                .map_err(|e| format!("WGI 控制器枚举失败：{e}"))?;
            let mut devices = Vec::new();
            for controller in controllers {
                let route = controller
                    .NonRoamableId()
                    .map_err(|e| format!("WGI 控制器路由标识不可用：{e}"))?
                    .to_string();
                if route.is_empty() {
                    continue; // Never substitute a name, VID/PID or list index.
                }
                let id = route_id(Backend::Wgi, lease, &route);
                if devices.iter().any(|device: &Device| device.id == id) {
                    continue;
                }
                let name = controller.DisplayName().ok().map(|s| s.to_string());
                devices.push(Device {
                    id,
                    name: name
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "WGI 控制器".into()),
                    container: None,
                    kind: Kind::Gamepad,
                    endpoint: Endpoint::Wgi(controller),
                });
            }
            Ok(devices)
        }
        Backend::XInput => {
            let mut devices = Vec::new();
            for slot in 0..XUSER_MAX_COUNT {
                let report = xinput_report(slot)?;
                if report.is_none_or(|r| r.BatteryType == BATTERY_TYPE_DISCONNECTED) {
                    continue;
                }
                devices.push(Device {
                    id: route_id(Backend::XInput, lease, &slot.to_string()),
                    name: format!("XInput 控制器槽位 {}", slot + 1),
                    container: None,
                    kind: Kind::Gamepad,
                    endpoint: Endpoint::XInput(slot),
                });
            }
            Ok(devices)
        }
    }
}

fn route_id(backend: Backend, lease: u64, route: &str) -> String {
    let namespace = match backend {
        Backend::Wgi => "wgi-route",
        Backend::XInput => "xinput-route",
    };
    battery::key(namespace, &format!("{lease}:{route}"))
}

fn current(generation: &AtomicU64, expected: u64) -> Result<(), String> {
    if generation.load(Ordering::Acquire) == expected {
        Ok(())
    } else {
        Err("控制器查询已取消或超时".into())
    }
}

impl Device {
    pub fn source(&self) -> Source {
        match self.endpoint {
            Endpoint::Wgi(_) => Source::Wgi,
            Endpoint::XInput(_) => Source::XInput,
        }
    }

    pub fn identity(&self) -> Evidence {
        Evidence::Route(self.id.clone())
    }

    pub fn query(&self, generation: &AtomicU64, expected: u64) -> Result<Reading, String> {
        current(generation, expected)?;
        let reading = match &self.endpoint {
            Endpoint::Wgi(controller) => {
                let report = controller
                    .TryGetBatteryReport()
                    .map_err(|e| format!("WGI 电池报告不可用：{e}"))?;
                // windows-rs represents a null IReference as an error. Nulls and
                // unavailable fields stay None; neither is silently converted to 0.
                let remaining = report
                    .RemainingCapacityInMilliwattHours()
                    .ok()
                    .and_then(|v| v.Value().ok());
                let full = report
                    .FullChargeCapacityInMilliwattHours()
                    .ok()
                    .and_then(|v| v.Value().ok());
                wgi_reading(remaining, full, report.Status().ok())
            }
            Endpoint::XInput(slot) => match xinput_report(*slot)? {
                Some(report) => xinput_reading(report.BatteryType, report.BatteryLevel),
                None => disconnected(),
            },
        };
        // Synchronous OS calls cannot be cancelled, but a late result cannot
        // cross the scheduler's generation boundary and become a fresh sample.
        current(generation, expected)?;
        Ok(reading)
    }
}

// Only this documented battery API is called. No input state, button capture,
// rumble, mode changes or XInputEnable calls are required.
fn xinput_report(slot: u32) -> Result<Option<XINPUT_BATTERY_INFORMATION>, String> {
    let mut report = XINPUT_BATTERY_INFORMATION::default();
    let status = unsafe { XInputGetBatteryInformation(slot, BATTERY_DEVTYPE_GAMEPAD, &mut report) };
    match status {
        ERROR_SUCCESS => Ok(Some(report)),
        ERROR_DEVICE_NOT_CONNECTED => Ok(None),
        error => Err(format!("XInput 电池查询失败：Windows 错误 {error}")),
    }
}

fn disconnected() -> Reading {
    let mut reading = Reading::new(Value::Unknown, Kind::Gamepad, Power::Unknown);
    reading.connected = false;
    reading
}

// WGI may synthesize 100/70/40/10 from four hardware levels. Even another
// ratio is not resolution evidence. Retain an ordinal until device validation
// establishes a finer resolution; Idle is not proof of completed charging.
fn wgi_reading(
    remaining: Option<i32>,
    full: Option<i32>,
    status: Option<BatteryStatus>,
) -> Reading {
    let value = match (remaining, full) {
        (Some(remaining), Some(full)) if full > 0 && remaining >= 0 && remaining <= full => {
            let scaled = i64::from(remaining) * 100;
            let full = i64::from(full);
            let level = if scaled <= full * 10 {
                Level::Critical
            } else if scaled <= full * 40 {
                Level::Low
            } else if scaled <= full * 70 {
                Level::Medium
            } else if scaled < full * 100 {
                Level::High
            } else {
                Level::Full
            };
            Value::Ordinal { level }
        }
        _ => Value::Unknown,
    };
    let power = match status {
        Some(BatteryStatus::Charging) => Power::Charging,
        Some(BatteryStatus::Discharging) => Power::Battery,
        _ => Power::Unknown,
    };
    Reading::new(value, Kind::Gamepad, power)
}

fn xinput_reading(kind: BATTERY_TYPE, level: BATTERY_LEVEL) -> Reading {
    match kind {
        BATTERY_TYPE_DISCONNECTED => disconnected(),
        BATTERY_TYPE_WIRED => Reading::new(Value::NoBattery, Kind::Gamepad, Power::Unknown),
        BATTERY_TYPE_ALKALINE | BATTERY_TYPE_NIMH => {
            let value = match level {
                BATTERY_LEVEL_EMPTY => Value::Ordinal {
                    level: Level::Critical,
                },
                BATTERY_LEVEL_LOW => Value::Ordinal { level: Level::Low },
                BATTERY_LEVEL_MEDIUM => Value::Ordinal {
                    level: Level::Medium,
                },
                BATTERY_LEVEL_FULL => Value::Ordinal { level: Level::Full },
                _ => Value::Unknown,
            };
            // XInput reports a level, not whether a rechargeable pack is charging.
            Reading::new(value, Kind::Gamepad, Power::Unknown)
        }
        _ => Reading::new(Value::Unknown, Kind::Gamepad, Power::Unknown),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::Input::XboxController::BATTERY_TYPE_UNKNOWN;

    #[test]
    fn nullable_and_invalid_wgi_capacities_never_become_percent_or_zero() {
        for (remaining, full) in [
            (None, None),
            (None, Some(100)),
            (Some(50), None),
            (Some(0), Some(0)),
            (Some(10), Some(-1)),
            (Some(-1), Some(100)),
            (Some(101), Some(100)),
            (Some(i32::MAX), Some(1)),
        ] {
            assert_eq!(wgi_reading(remaining, full, None).value, Value::Unknown);
        }
        assert_eq!(
            wgi_reading(Some(0), Some(100), None).value,
            Value::Ordinal {
                level: Level::Critical
            }
        );
        assert_eq!(
            wgi_reading(Some(i32::MAX), Some(i32::MAX), None).value,
            Value::Ordinal { level: Level::Full }
        );
    }

    #[test]
    fn wgi_coarse_and_unproven_finer_ratios_are_ordinal() {
        for (remaining, level) in [
            (10, Level::Critical),
            (40, Level::Low),
            (70, Level::Medium),
            (100, Level::Full),
            (55, Level::Medium),
            (99, Level::High),
        ] {
            let reading = wgi_reading(Some(remaining), Some(100), Some(BatteryStatus::Idle));
            assert_eq!(reading.value, Value::Ordinal { level });
            assert_eq!(reading.percent(), None);
            assert_eq!(reading.power, Power::Unknown);
        }
        assert_eq!(
            wgi_reading(None, None, Some(BatteryStatus::Charging)).power,
            Power::Charging
        );
        assert_eq!(
            wgi_reading(Some(100), Some(100), Some(BatteryStatus::Discharging)).power,
            Power::Battery
        );
        assert_eq!(
            wgi_reading(None, None, Some(BatteryStatus::NotPresent)).value,
            Value::Unknown
        );
    }

    #[test]
    fn xinput_type_matrix_does_not_trust_invalid_or_unknown_types() {
        for kind in [BATTERY_TYPE_ALKALINE, BATTERY_TYPE_NIMH] {
            for (level, expected) in [
                (BATTERY_LEVEL_EMPTY, Level::Critical),
                (BATTERY_LEVEL_LOW, Level::Low),
                (BATTERY_LEVEL_MEDIUM, Level::Medium),
                (BATTERY_LEVEL_FULL, Level::Full),
            ] {
                let reading = xinput_reading(kind, level);
                assert_eq!(reading.value, Value::Ordinal { level: expected });
                assert_eq!(reading.percent(), None);
                assert_eq!(reading.power, Power::Unknown);
            }
            assert_eq!(
                xinput_reading(kind, BATTERY_LEVEL(255)).value,
                Value::Unknown
            );
        }
        for level in [BATTERY_LEVEL_EMPTY, BATTERY_LEVEL_FULL, BATTERY_LEVEL(255)] {
            assert_eq!(
                xinput_reading(BATTERY_TYPE_WIRED, level).value,
                Value::NoBattery
            );
            assert_eq!(
                xinput_reading(BATTERY_TYPE_UNKNOWN, level).value,
                Value::Unknown
            );
            assert_eq!(xinput_reading(BATTERY_TYPE(4), level).value, Value::Unknown);
            let absent = xinput_reading(BATTERY_TYPE_DISCONNECTED, level);
            assert_eq!(absent.value, Value::Unknown);
            assert!(!absent.connected);
        }
    }

    #[test]
    fn experimental_routes_never_claim_persisted_physical_identity() {
        let device = Device {
            id: route_id(Backend::XInput, 1, "0"),
            name: "same name".into(),
            container: None,
            kind: Kind::Gamepad,
            endpoint: Endpoint::XInput(0),
        };
        assert!(matches!(device.identity(), Evidence::Route(ref id) if id == &device.id));
        assert!(device.container.is_none());
        assert_ne!(device.id, route_id(Backend::XInput, 2, "0"));
        assert_ne!(device.id, route_id(Backend::XInput, 1, "1"));
        assert_ne!(device.id, route_id(Backend::Wgi, 1, "0"));
        assert!(enumerate().unwrap().is_empty());
    }

    #[test]
    fn cancelled_generation_never_queries_hardware() {
        let device = Device {
            id: "test".into(),
            name: "test".into(),
            container: None,
            kind: Kind::Gamepad,
            endpoint: Endpoint::XInput(u32::MAX),
        };
        assert!(device.query(&AtomicU64::new(2), 1).is_err());
    }
}
