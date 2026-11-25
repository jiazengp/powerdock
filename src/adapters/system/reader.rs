//! Per-transport connection handles, bounded reads and standard fallback.
use super::*;

#[derive(Default)]
pub struct Link {
    pub metadata: Option<MetadataBattery>,
    sennheiser_identity: Option<sennheiser::Identity>,
    sennheiser_retry: Option<Instant>,
    classic: LinkHandle,
    low_energy: LinkHandle,
    pub components: Vec<battery::Component>,
    pub observation: Option<(u64, Instant)>,
    pub subscribed: bool,
    pub wake: Option<gatt::Wake>,
    pub force_uncached: bool,
}
#[derive(Default)]
enum LinkHandle {
    Classic(BluetoothDevice),
    LowEnergy(BluetoothLEDevice, gatt::Session),
    #[default]
    None,
}
impl Drop for LinkHandle {
    fn drop(&mut self) {
        match self {
            Self::Classic(d) => {
                let _ = d.Close();
            }
            Self::LowEnergy(d, session) => {
                *session = gatt::Session::default();
                let _ = d.Close();
            }
            Self::None => {}
        }
    }
}
impl LinkHandle {
    fn connected(&mut self) -> Option<bool> {
        let status = match self {
            Self::Classic(d) => d.ConnectionStatus().ok(),
            Self::LowEnergy(d, _) => d.ConnectionStatus().ok(),
            Self::None => None,
        };
        let connected = status.map(|s| s == BluetoothConnectionStatus::Connected);
        if connected != Some(true) {
            // Drop stale service handles; each transport reconnects independently.
            *self = Self::None;
        }
        connected
    }
    fn kind(&self) -> Kind {
        match self {
            Self::Classic(d) => d
                .ClassOfDevice()
                .and_then(|c| c.RawValue())
                .map_or(Kind::Other, classic_kind),
            Self::LowEnergy(d, _) => d
                .Appearance()
                .and_then(|a| a.RawValue())
                .map_or(Kind::Other, ble_kind),
            Self::None => Kind::Other,
        }
    }
}

// Real ConnectionStatus values gate every path; AEP hints only affect discovery.
pub(super) fn read_connected<T>(
    connected: [Option<bool>; 2],
    mut read: impl FnMut(bool) -> Result<T, String>,
) -> Result<T, String> {
    let mut error = if connected.contains(&Some(false)) {
        "蓝牙外设未连接 · 系统电量可能是缓存".to_owned()
    } else {
        "Windows 有电量缓存，但无法确认外设当前连接".to_owned()
    };
    for (index, status) in connected.into_iter().enumerate() {
        if status == Some(true) {
            match read(index == 1) {
                Ok(reading) => return Ok(reading),
                Err(next) => error = next,
            }
        }
    }
    Err(error)
}

impl Peripheral {
    #[cfg(test)]
    pub(crate) fn test_paired(container: &str, connected_hint: bool) -> Self {
        Self {
            standard_hid: None,
            id: battery::key("bt", container),
            name: container.into(),
            container: Some(container.into()),
            kind: Kind::Mouse,
            connected_hint,
            nodes: Vec::new(),
            classic: None,
            low_energy: None,
        }
    }
    // A headset's battery can live on a sibling HFP/audio node, not its Bluetooth root.
    // Query only the same PnP container; never scan unrelated device battery properties.
    fn container_battery(&self, generation: &AtomicU64, expected: u64) -> Option<u8> {
        use windows::{
            Devices::Enumeration::{DeviceInformation, DeviceInformationKind},
            Foundation::{IPropertyValue, PropertyType},
            core::{HSTRING, Interface},
        };
        let encoded = self.container.as_deref()?;
        if encoded.len() != 32 {
            return None;
        }
        let mut b = [0u8; 16];
        for (i, byte) in b.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&encoded[i * 2..i * 2 + 2], 16).ok()?;
        }
        let guid = windows::core::GUID::from_values(
            u32::from_le_bytes(b[..4].try_into().ok()?),
            u16::from_le_bytes(b[4..6].try_into().ok()?),
            u16::from_le_bytes(b[6..8].try_into().ok()?),
            b[8..].try_into().ok()?,
        );
        let filter = HSTRING::from(format!("System.Devices.ContainerId:=\"{{{guid:?}}}\""));
        let keys = [
            "System.Devices.BatteryLife",
            "{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2",
        ];
        let requested: windows_collections::IIterable<HSTRING> = keys
            .iter()
            .map(|k| HSTRING::from(*k))
            .collect::<Vec<_>>()
            .into();
        let operation = DeviceInformation::FindAllAsyncWithKindAqsFilterAndAdditionalProperties(
            &filter,
            &requested,
            DeviceInformationKind::Device,
        )
        .ok()?;
        let deadline = Instant::now() + Duration::from_millis(600);
        let nodes = crate::winrt::wait(Ok(operation), deadline, generation, expected).ok()?;
        for key in keys {
            for i in 0..nodes.Size().ok()?.min(64) {
                let Some(value) = nodes
                    .GetAt(i)
                    .ok()
                    .and_then(|n| n.Properties().ok())
                    .and_then(|p| p.Lookup(&HSTRING::from(key)).ok())
                    .and_then(|v| v.cast::<IPropertyValue>().ok())
                else {
                    continue;
                };
                let number = match value.Type().ok() {
                    Some(PropertyType::UInt8) => value.GetUInt8().ok().map(u32::from),
                    Some(PropertyType::UInt32) => value.GetUInt32().ok(),
                    _ => None,
                };
                if let Some(level) = number.filter(|v| *v <= 100) {
                    return Some(level as u8);
                }
            }
        }
        None
    }
    #[cfg(test)]
    pub(crate) fn test_classic_headset(container: &str) -> Self {
        let mut p = Self::test_paired(container, false);
        p.kind = Kind::Headset;
        p.classic = Some(Endpoint {
            address: 1,
            connected_hint: false,
            address_type: None,
        });
        p
    }
    pub fn sources(&self) -> Vec<Source> {
        let mut sources = vec![Source::Windows];
        if self.standard_hid.is_some() {
            sources.push(Source::HidStandard);
        }
        if self.low_energy.is_some() {
            sources.push(Source::BluetoothGatt);
        }
        if self.sennheiser_candidate() {
            sources.push(Source::Sennheiser);
        }
        sources
    }
    pub fn query(
        &self,
        link: &mut Link,
        source: &mut Source,
        connection: &mut Option<bool>,
        generation: &AtomicU64,
        expected: u64,
        policy: &crate::settings::ProviderPreferences,
    ) -> Result<Reading, String> {
        if generation.load(Ordering::Acquire) != expected {
            return Err("设备会话已变更".into());
        }
        link.components.clear();
        link.observation = None;
        link.subscribed = false;
        for (le, endpoint, handle) in [
            (false, &self.classic, &mut link.classic),
            (true, &self.low_energy, &mut link.low_energy),
        ] {
            let Some(endpoint) = endpoint else { continue };
            if !matches!(handle, LinkHandle::None) {
                continue;
            }
            let deadline = Instant::now() + Duration::from_millis(600);
            if le {
                let operation = if let Some(address_type) = endpoint.address_type {
                    BluetoothLEDevice::FromBluetoothAddressWithBluetoothAddressTypeAsync(
                        endpoint.address,
                        address_type,
                    )
                } else {
                    BluetoothLEDevice::FromBluetoothAddressAsync(endpoint.address)
                };
                if let Ok(device) = crate::winrt::wait(operation, deadline, generation, expected) {
                    *handle = LinkHandle::LowEnergy(device, gatt::Session::default());
                }
            } else if let Ok(device) = crate::winrt::wait(
                BluetoothDevice::FromBluetoothAddressAsync(endpoint.address),
                deadline,
                generation,
                expected,
            ) {
                *handle = LinkHandle::Classic(device);
            }
        }
        let connected = [link.classic.connected(), link.low_energy.connected()];
        if connected[0] == Some(false) {
            link.sennheiser_identity = None;
            link.sennheiser_retry = None;
        }
        *connection = if connected.contains(&Some(true)) {
            Some(true)
        } else if connected.contains(&Some(false)) {
            Some(false)
        } else {
            None
        };
        let metadata = link.metadata.as_ref().filter(|m| m.current()).cloned();
        let mut cached = if policy.source(Source::Windows) {
            metadata
                .as_ref()
                .and_then(|m| m.percent)
                .or_else(|| battery_value(&self.nodes))
        } else {
            None
        };
        let mut container_queried = false;
        let result = read_connected(connected, |le| {
            if generation.load(Ordering::Acquire) != expected {
                return Err("设备会话已变更".into());
            }
            let mut protocol_error = None;
            if !le
                && policy.source(Source::Sennheiser)
                && self.sennheiser_candidate()
                && link
                    .sennheiser_retry
                    .is_none_or(|until| Instant::now() >= until)
                && let Some(endpoint) = &self.classic
            {
                *source = Source::Sennheiser;
                match sennheiser::read(
                    endpoint.address,
                    generation,
                    expected,
                    &mut link.sennheiser_identity,
                    policy.profile(crate::settings::Profile::SennheiserFamilyExperimental),
                ) {
                    Ok(observation) => {
                        link.sennheiser_retry = None;
                        link.components = observation.components;
                        return Ok(observation.reading);
                    }
                    Err(error) => {
                        crate::diagnostics::debug(
                            "provider_query_failed",
                            serde_json::json!({"provider":"sennheiser_gaia","error":error}),
                        );
                        link.sennheiser_retry = Some(Instant::now() + Duration::from_secs(30));
                        protocol_error = Some(error);
                    }
                }
            }
            let handle = if le {
                &mut link.low_energy
            } else {
                &mut link.classic
            };
            if policy.source(Source::BluetoothGatt)
                && let LinkHandle::LowEnergy(device, session) = handle
            {
                if link.force_uncached {
                    session.require_uncached_read();
                }
                match session.read(
                    device,
                    self.kind,
                    policy.profile(crate::settings::Profile::Bas11Experimental),
                    link.wake.clone(),
                    generation,
                    expected,
                ) {
                    Ok(frame) => {
                        *source = Source::BluetoothGatt;
                        link.components = frame.components;
                        link.observation = Some((frame.at, frame.clock));
                        link.subscribed = frame.subscribed;
                        return Ok(frame.reading);
                    }
                    Err(error) => protocol_error = Some(error),
                }
            }
            if policy.source(Source::Windows) && cached.is_none() && !container_queried {
                cached = self.container_battery(generation, expected);
                container_queried = true;
            }
            let Some(percent) = cached else {
                return Err(
                    protocol_error.unwrap_or_else(|| "已连接 · Windows 未提供可读取的电量".into())
                );
            };
            *source = Source::Windows;
            if let Some(meta) = metadata.as_ref().filter(|m| m.percent.is_some()) {
                link.observation = Some((meta.at, meta.clock));
            }
            let native_power = || {
                self.nodes
                    .iter()
                    .find_map(|n| number(*n, &CHARGING))
                    .map_or(Power::Unknown, |state| match state {
                        0 => Power::Battery,
                        1 => Power::Charging,
                        _ => Power::Unknown,
                    })
            };
            let power = metadata
                .as_ref()
                .map_or_else(native_power, |m| m.power_or(native_power));
            let kind = handle.kind();
            Ok(Reading {
                value: crate::battery::Value::reported(percent),
                kind: if kind == Kind::Other { self.kind } else { kind },
                power,
                connected: true,
            })
        });
        if policy.source(Source::HidStandard)
            && let Some(hid) = &self.standard_hid
        {
            return standard_hid_fallback(result, source, connection, || {
                hid.query(generation, expected)
            });
        }
        result
    }
}
// A container association is identity evidence, not a dependency on the other
// transport's connection status. HID can independently answer when BT cannot.
pub(super) fn standard_hid_fallback(
    result: Result<Reading, String>,
    source: &mut Source,
    connection: &mut Option<bool>,
    read: impl FnOnce() -> Result<Reading, String>,
) -> Result<Reading, String> {
    if result.is_ok() && *source != Source::Windows {
        return result;
    }
    match read() {
        Ok(reading) => {
            *source = Source::HidStandard;
            *connection = Some(reading.connected);
            Ok(reading)
        }
        Err(error) => result.or(Err(error)),
    }
}
