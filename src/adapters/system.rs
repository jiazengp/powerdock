mod endpoints;
mod properties;
use super::{gatt, hid_standard, sennheiser};
pub(crate) use endpoints::container_event_key;
#[cfg(test)]
use endpoints::{endpoint_address, guid_key};
use endpoints::{endpoint_property, supplement_endpoints, supplement_info};
pub use properties::container;
use properties::{instance, integer, number, property, text};
mod reader;
pub use reader::Link;
#[cfg(test)]
mod kind_tests;
#[cfg(test)]
mod tests;

// Paired Bluetooth/PnP discovery and scoped metadata.
use crate::battery::{self, Kind, Power, Reading, Source};
use std::{
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use windows::Devices::Bluetooth::{
    BluetoothAddressType, BluetoothConnectionStatus, BluetoothDevice, BluetoothLEDevice,
};
use windows_sys::{
    Win32::{
        Devices::{DeviceAndDriverInstallation::*, Properties::*},
        Foundation::*,
    },
    core::GUID,
};

const MOUSE_CLASS: GUID = GUID::from_u128(0x4d36e96f_e325_11ce_bfc1_08002be10318);
// Public Windows SDK bthdef.h property, also exposed by windows-sys Wdk::Devices::Bluetooth.
const BLUETOOTH_CLASS: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0x2bd67d8b_8beb_48d5_87e0_6cda3428040a),
    pid: 10,
};
// Older Bluetooth drivers expose the same DWORD at the deprecated PID 4.
const BLUETOOTH_CLASS_LEGACY: DEVPROPKEY = DEVPROPKEY {
    fmtid: BLUETOOTH_CLASS.fmtid,
    pid: 4,
};
const BATTERY: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0x49cd1f76_5626_4b17_a4e8_18b4aa1a2213),
    pid: 10,
};
const CHARGING: DEVPROPKEY = DEVPROPKEY {
    fmtid: BATTERY.fmtid,
    pid: 11,
};
// Compatibility fallback observed in HaloBattery; not a documented Microsoft contract.
const BLUETOOTH_BATTERY: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5),
    pid: 2,
};

#[derive(Clone, PartialEq, Eq)]
pub struct Peripheral {
    pub standard_hid: Option<hid_standard::Device>,
    pub id: String,
    pub name: String,
    pub container: Option<String>,
    pub kind: Kind,
    pub connected_hint: bool,
    nodes: Vec<u32>,
    classic: Option<Endpoint>,
    low_energy: Option<Endpoint>,
}
#[derive(Clone, PartialEq, Eq)]
struct Endpoint {
    address: u64,
    connected_hint: bool,
    address_type: Option<BluetoothAddressType>,
}

impl Peripheral {
    fn sennheiser_candidate(&self) -> bool {
        self.classic.is_some() && self.kind == Kind::Headset
    }
    fn has_gaia_service_node(&self) -> bool {
        self.sennheiser_candidate()
            && self
                .nodes
                .iter()
                .filter_map(|node| instance(*node))
                .any(|id| sennheiser::gaia_service_node(&id))
    }
}

fn merge_device(devices: &mut Vec<Peripheral>, discovered: Peripheral) {
    if let Some(existing) = devices.iter_mut().find(|d| d.id == discovered.id) {
        existing.connected_hint |= discovered.connected_hint;
        existing.container = existing.container.take().or(discovered.container);
        if existing.kind == Kind::Other {
            existing.kind = discovered.kind;
        }
        for (slot, endpoint) in [
            (&mut existing.classic, discovered.classic),
            (&mut existing.low_energy, discovered.low_energy),
        ] {
            if let Some(endpoint) = endpoint {
                if let Some(previous) = slot {
                    previous.connected_hint |= endpoint.connected_hint;
                    previous.address_type = endpoint.address_type.or(previous.address_type);
                } else {
                    *slot = Some(endpoint);
                }
            }
        }
        for node in discovered.nodes {
            if !existing.nodes.contains(&node) {
                existing.nodes.push(node);
            }
        }
    } else {
        devices.push(discovered);
    }
}
struct Set(HDEVINFO);
impl Drop for Set {
    fn drop(&mut self) {
        unsafe {
            SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

fn bluetooth_address(instance: &str) -> Option<(u64, bool)> {
    let upper = instance.to_ascii_uppercase();
    let (suffix, le) = if let Some(s) = upper.strip_prefix("BTHLE\\DEV_") {
        (s, true)
    } else {
        (upper.strip_prefix("BTHENUM\\DEV_")?, false)
    };
    let address = suffix.split('\\').next()?;
    if address.len() != 12 || !address.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some((u64::from_str_radix(address, 16).ok()?, le))
}

pub fn enumerate() -> Result<Vec<Peripheral>, String> {
    enumerate_with(None)
}
pub fn enumerate_with(
    watcher: Option<&crate::discovery::Watchers>,
) -> Result<Vec<Peripheral>, String> {
    unsafe {
        let mut devices: Vec<Peripheral> = Vec::new();
        for enumerator in [
            None,
            Some("BTHENUM"),
            Some("BTHLE"),
            Some("BTHLEDEVICE"),
            Some("BTHHFENUM"),
        ] {
            let identifier = enumerator.map(crate::settings::wide);
            let raw = SetupDiGetClassDevsW(
                if enumerator.is_none() {
                    &MOUSE_CLASS
                } else {
                    null()
                },
                identifier.as_ref().map_or(null(), |s| s.as_ptr()),
                null_mut(),
                DIGCF_PRESENT
                    | if enumerator.is_some() {
                        DIGCF_ALLCLASSES
                    } else {
                        0
                    },
            );
            if raw == -1 {
                if enumerator.is_none() {
                    return Err("无法枚举系统外设元数据".into());
                }
                continue;
            }
            let set = Set(raw);
            for index in 0..512 {
                let mut info: SP_DEVINFO_DATA = zeroed();
                info.cbSize = size_of::<SP_DEVINFO_DATA>() as u32;
                if SetupDiEnumDeviceInfo(set.0, index, &mut info) == 0 {
                    if GetLastError() == ERROR_NO_MORE_ITEMS {
                        break;
                    }
                    return Err("系统鼠标枚举失败".into());
                }
                let Some(_) = instance(info.DevInst) else {
                    continue;
                };
                let mut node = info.DevInst;
                let mut nodes = Vec::new();
                let mut address = None;
                let mut physical = None;
                let mut name = text(node, &DEVPKEY_Device_FriendlyName)
                    .or_else(|| text(node, &DEVPKEY_Device_BusReportedDeviceDesc))
                    .unwrap_or_else(|| "蓝牙外设".into());
                for _ in 0..12 {
                    nodes.push(node);
                    if physical.is_none() {
                        physical = container(node);
                    }
                    if let Some(id) = instance(node)
                        && let Some(bt) = bluetooth_address(&id)
                    {
                        address = Some(bt);
                        if let Some(value) = text(node, &DEVPKEY_Device_FriendlyName) {
                            name = value;
                        }
                        physical = container(node).or(physical);
                        break; // Never inherit properties/identity from the Bluetooth adapter.
                    }
                    let mut parent = 0;
                    if CM_Get_Parent(&mut parent, node, 0) != CR_SUCCESS || parent == node {
                        break;
                    }
                    node = parent;
                }
                let kind = nodes
                    .iter()
                    .find_map(|node| {
                        let (kind, data) = property(*node, &DEVPKEY_DeviceContainer_Category)?;
                        (category_kind(kind, &data) == Kind::Phone).then_some(Kind::Phone)
                    })
                    .unwrap_or_else(|| {
                        nodes
                            .iter()
                            .find_map(|node| {
                                number(*node, &BLUETOOTH_CLASS)
                                    .or_else(|| number(*node, &BLUETOOTH_CLASS_LEGACY))
                            })
                            .map_or(Kind::Other, classic_kind)
                    });
                let Some((mac, le)) = address else { continue };
                let id = battery::key("bt", &format!("{mac:012x}"));
                let endpoint = Endpoint {
                    address: mac,
                    connected_hint: false,
                    address_type: None,
                };
                merge_device(
                    &mut devices,
                    Peripheral {
                        standard_hid: None,
                        id,
                        name,
                        container: physical,
                        kind,
                        connected_hint: false,
                        nodes,
                        classic: (!le).then_some(endpoint.clone()),
                        low_energy: le.then_some(endpoint),
                    },
                );
            }
        }
        if let Some(watcher) = watcher {
            let complete = watcher.initial_complete();
            watcher.with_snapshot(|infos| {
                let mut paired = Vec::new();
                for (le, info) in infos {
                    supplement_info(&mut paired, *le, info);
                }
                if complete {
                    retain_paired_candidates(&mut devices, &paired);
                }
                for info in paired {
                    merge_device(&mut devices, info);
                }
            })?;
        } else {
            supplement_endpoints(&mut devices);
        }
        // Filter after merging so an unclassified sibling cannot lose its endpoint.
        devices.retain(|d| {
            d.connected_hint
                || (!d.nodes.is_empty() && (d.kind != Kind::Other || exposes_battery(&d.nodes)))
        });
        Ok(devices)
    }
}
// A paired-AEP selector is not an exhaustive inventory of present Classic
// endpoints on every driver. Retain scoped brand-probe candidates; the runtime
// ConnectionStatus gate still decides whether any protocol I/O may run.
fn retain_paired_candidates(devices: &mut Vec<Peripheral>, paired: &[Peripheral]) {
    crate::diagnostics::debug(
        "paired_inventory_coverage",
        serde_json::json!({
            "pnp_classic_headsets":devices.iter().filter(|d|d.sennheiser_candidate()).count(),
            "paired_classic_headsets":paired.iter().filter(|d|d.sennheiser_candidate()).count()
        }),
    );
    devices.retain(|d| paired.iter().any(|p| p.id == d.id) || d.sennheiser_candidate());
}
fn exposes_battery(nodes: &[u32]) -> bool {
    [&BATTERY, &BLUETOOTH_BATTERY]
        .iter()
        .any(|key| nodes.iter().any(|node| number(*node, key).is_some()))
}

pub fn diagnose() -> Result<serde_json::Value, String> {
    Ok(serde_json::Value::Array(enumerate()?.iter().map(|device| {
        serde_json::json!({
            "id":device.id, "name":device.name, "kind":device.kind,
            "transport":match (&device.classic, &device.low_energy) {
                (Some(_), Some(_)) => "bluetooth_dual",
                (_, Some(_)) => "bluetooth_le",
                _ => "bluetooth_classic",
            },
            "connected_hint":device.connected_hint,
            "pnp_node_count":device.nodes.len(), "has_gaia_service_node":device.has_gaia_service_node(), "has_container_identity":device.container.is_some(),
            "cached_percent":battery_value(&device.nodes), "source_cache":true,
            "battery_protocol_queried":false,
        })
    }).collect()))
}

fn percentage(kind: u32, data: &[u8]) -> Option<u8> {
    integer(kind, data)
        .filter(|value| *value <= 100)
        .map(|value| value as u8)
}
fn battery_value(nodes: &[u32]) -> Option<u8> {
    for key in [&BATTERY, &BLUETOOTH_BATTERY] {
        for node in nodes {
            if let Some((kind, data)) = property(*node, key)
                && let Some(value) = percentage(kind, &data)
            {
                return Some(value);
            }
        }
    }
    None
}
#[derive(Clone)]
pub struct MetadataBattery {
    pub percent: Option<u8>,
    pub power: Option<Power>,
    pub at: u64,
    pub clock: Instant,
}
impl MetadataBattery {
    fn from_properties(percent: Option<u8>, charging: Option<u8>) -> Self {
        Self {
            percent: percent.filter(|p| *p <= 100),
            // Missing differs from explicit Unknown/NotCharging. Neither of the
            // latter states proves discharge or Full.
            power: charging.map(|c| {
                if c == 1 {
                    Power::Charging
                } else {
                    Power::Unknown
                }
            }),
            at: crate::settings::now(),
            clock: Instant::now(),
        }
    }
    fn power_or(&self, fallback: impl FnOnce() -> Power) -> Power {
        self.power.unwrap_or_else(fallback)
    }
    fn current(&self) -> bool {
        self.clock.elapsed() < battery::FRESH_FOR
    }
}
pub(crate) fn container_metadata(
    info: &windows::Devices::Enumeration::DeviceInformation,
) -> Option<(String, MetadataBattery)> {
    use windows::Foundation::PropertyType;
    let key = container_event_key(&info.Id().ok()?.to_string())?;
    let byte = |name| {
        endpoint_property(info, name)
            .filter(|v| v.Type().ok() == Some(PropertyType::UInt8))
            .and_then(|v| v.GetUInt8().ok())
    };
    Some((
        key,
        MetadataBattery::from_properties(
            byte("System.Devices.BatteryLife"),
            byte("System.Devices.ChargingState"),
        ),
    ))
}
// Category is a string list; a friendly device name is never type evidence.
fn category_kind(kind: u32, data: &[u8]) -> Kind {
    if kind != DEVPROP_TYPE_STRING_LIST || !data.len().is_multiple_of(2) {
        return Kind::Other;
    }
    let units: Vec<u16> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    if units.split(|u| *u == 0).any(|value| {
        let value = String::from_utf16_lossy(value).to_ascii_lowercase();
        value == "communication.phone" || value.starts_with("communication.phone.")
    }) {
        Kind::Phone
    } else {
        Kind::Other
    }
}
fn classic_kind(raw: u32) -> Kind {
    let major = (raw >> 8) & 0x1f;
    let minor = (raw >> 2) & 0x3f;
    if major == 2 {
        return Kind::Phone;
    }
    if major == 4 && [1, 2, 6].contains(&minor) {
        return Kind::Headset;
    }
    if major == 5 {
        match (raw >> 6) & 3 {
            1 | 3 => return Kind::Keyboard,
            2 => return Kind::Mouse,
            _ => {}
        }
        if [1, 2].contains(&((raw >> 2) & 0x0f)) {
            return Kind::Gamepad;
        }
    }
    Kind::Other
}
fn ble_kind(raw: u16) -> Kind {
    match raw {
        0x40 => Kind::Phone, // Bluetooth SIG generic phone appearance.
        0x3c1 => Kind::Keyboard,
        0x3c2 => Kind::Mouse,
        0x3c3 | 0x3c4 => Kind::Gamepad,
        0x941..=0x944 => Kind::Headset,
        _ => Kind::Other,
    }
}
#[cfg(test)]
mod percentage_tests;
