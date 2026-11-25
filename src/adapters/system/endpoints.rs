//! Supplement PnP discovery with paired Windows Bluetooth endpoints.
use super::*;

pub(super) fn endpoint_address(value: &str) -> Option<u64> {
    let compact: String = value.chars().filter(|c| *c != ':' && *c != '-').collect();
    if compact.len() != 12 || !compact.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(&compact, 16)
        .ok()
        .filter(|address| *address != 0)
}
pub(crate) fn container_event_key(id: &str) -> Option<String> {
    let id = id
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))
        .unwrap_or(id);
    windows::core::GUID::try_from(id).ok().and_then(guid_key)
}
pub(super) fn guid_key(guid: windows::core::GUID) -> Option<String> {
    let bytes: Vec<u8> = guid
        .data1
        .to_le_bytes()
        .into_iter()
        .chain(guid.data2.to_le_bytes())
        .chain(guid.data3.to_le_bytes())
        .chain(guid.data4)
        .collect();
    if bytes.iter().all(|b| *b == 0) {
        return None;
    }
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub(super) fn endpoint_property(
    info: &windows::Devices::Enumeration::DeviceInformation,
    key: &str,
) -> Option<windows::Foundation::IPropertyValue> {
    use windows::core::{HSTRING, Interface};
    info.Properties()
        .ok()?
        .Lookup(&HSTRING::from(key))
        .ok()?
        .cast()
        .ok()
}
// SetupAPI can miss paired BLE devices with only a GATT Battery Service. Use
// paired AEP selectors as a supplement, never advertisement scanning or pairing.
pub(super) fn supplement_endpoints(devices: &mut Vec<Peripheral>) {
    use windows::{
        Devices::Enumeration::{DeviceInformation, DeviceInformationKind},
        Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
        core::HSTRING,
    };
    let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    struct Apartment(bool);
    impl Drop for Apartment {
        fn drop(&mut self) {
            if self.0 {
                unsafe {
                    CoUninitialize();
                }
            }
        }
    }
    let _apartment = Apartment(initialized);
    let requested: windows_collections::IIterable<HSTRING> = [
        "System.Devices.Aep.IsConnected",
        "System.Devices.Aep.DeviceAddress",
        "System.Devices.Aep.ContainerId",
        "System.Devices.Aep.Bluetooth.Le.Appearance",
        "System.Devices.Aep.Bluetooth.Le.AddressType",
    ]
    .into_iter()
    .map(HSTRING::from)
    .collect::<Vec<_>>()
    .into();
    let deadline = Instant::now() + Duration::from_millis(1600);
    let generation = AtomicU64::new(1);
    let operations: Vec<_> = [
        (
            false,
            BluetoothDevice::GetDeviceSelectorFromPairingState(true),
        ),
        (
            true,
            BluetoothLEDevice::GetDeviceSelectorFromPairingState(true),
        ),
    ]
    .into_iter()
    .filter_map(|(le, selector)| {
        let selector = selector.ok()?;
        Some((
            le,
            DeviceInformation::FindAllAsyncWithKindAqsFilterAndAdditionalProperties(
                &selector,
                &requested,
                DeviceInformationKind::AssociationEndpoint,
            ),
        ))
    })
    .collect();
    for (le, operation) in operations {
        let Ok(endpoints) = crate::winrt::wait(operation, deadline, &generation, 1) else {
            continue;
        };
        for index in 0..endpoints.Size().unwrap_or(0).min(128) {
            let Ok(info) = endpoints.GetAt(index) else {
                continue;
            };
            supplement_info(devices, le, &info);
        }
    }
}
pub(super) fn supplement_info(
    devices: &mut Vec<Peripheral>,
    le: bool,
    info: &windows::Devices::Enumeration::DeviceInformation,
) {
    use windows::Foundation::PropertyType;
    let connected = endpoint_property(info, "System.Devices.Aep.IsConnected")
        .filter(|p| p.Type().ok() == Some(PropertyType::Boolean))
        .and_then(|p| p.GetBoolean().ok())
        .unwrap_or(false);
    let Some(address) = endpoint_property(info, "System.Devices.Aep.DeviceAddress")
        .filter(|p| p.Type().ok() == Some(PropertyType::String))
        .and_then(|p| p.GetString().ok())
        .and_then(|s| endpoint_address(&s.to_string()))
    else {
        return;
    };
    let container = endpoint_property(info, "System.Devices.Aep.ContainerId")
        .filter(|p| p.Type().ok() == Some(PropertyType::Guid))
        .and_then(|p| p.GetGuid().ok())
        .and_then(guid_key);
    let kind = endpoint_property(info, "System.Devices.Aep.Bluetooth.Le.Appearance")
        .filter(|p| p.Type().ok() == Some(PropertyType::UInt16))
        .and_then(|p| p.GetUInt16().ok())
        .map_or(Kind::Other, ble_kind);
    let address_type = endpoint_property(info, "System.Devices.Aep.Bluetooth.Le.AddressType")
        .filter(|p| p.Type().ok() == Some(PropertyType::UInt8))
        .and_then(|p| p.GetUInt8().ok())
        .and_then(|value| match value {
            0 => Some(BluetoothAddressType::Public),
            1 => Some(BluetoothAddressType::Random),
            _ => None,
        });
    let id = battery::key("bt", &format!("{address:012x}"));
    {
        let endpoint = Endpoint {
            address,
            connected_hint: connected,
            address_type: if le { address_type } else { None },
        };
        merge_device(
            devices,
            Peripheral {
                standard_hid: None,
                id,
                name: info
                    .Name()
                    .map(|s| s.to_string().chars().take(64).collect())
                    .unwrap_or_else(|_| "蓝牙外设".into()),
                container,
                kind,
                connected_hint: connected,
                nodes: Vec::new(),
                classic: (!le).then_some(endpoint.clone()),
                low_energy: le.then_some(endpoint),
            },
        );
    }
}
