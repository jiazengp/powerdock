//! Read-only HID++ provider backed by the pinned OpenLogi protocol crate.
//!
//! Receiver routes are discoverable for diagnostics but intentionally ineligible
//! for automatic monitoring until pairing replacement has hardware evidence.
mod features;
mod session;
mod transport;
pub use session::Session;

use crate::{
    adapters::device,
    battery::{self, Kind, Reading},
    identity::Evidence,
};
use std::{collections::BTreeMap, fmt};

pub const RECEIVER_PAUSED: &str = "unsupported: HID++ receiver slots require verified pairing-replacement identity; automatic monitoring is paused";

#[derive(Clone, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub container: Option<String>,
    pub kind: Kind,
    group: String,
    slot: u8,
    receiver: bool,
    endpoints: Vec<device::Device>,
}
impl fmt::Debug for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HidppDevice")
            .field("id", &self.id)
            .field("slot", &self.slot)
            .field("receiver", &self.receiver)
            .finish()
    }
}
impl Device {
    pub fn gate_key(&self) -> String {
        self.group.clone()
    }
    pub fn is_receiver(&self) -> bool {
        self.receiver
    }
    pub fn monitor_eligible(&self) -> bool {
        !self.is_receiver()
    }
    pub fn matches_interface(&self, interfaces: &[String]) -> bool {
        self.endpoints
            .iter()
            .any(|d| device::matches_interface_path(&d.path, interfaces))
    }
}
pub(crate) fn eligible_collection(d: &device::Device) -> bool {
    d.vendor_id == openlogi_device_registry::LOGITECH_VENDOR_ID
        && d.usage_page == 0xff00
        && matches!(
            (d.usage, d.input_bytes, d.output_bytes),
            (1, 7, 7) | (2, 20, 20)
        )
}
/// Metadata only: no device opens, ping, register reads, or output reports.
pub fn enumerate_from(interfaces: &[device::Device]) -> Vec<Device> {
    let mut groups: BTreeMap<String, Vec<device::Device>> = BTreeMap::new();
    for d in interfaces.iter().filter(|d| eligible_collection(d)) {
        let route = d
            .container
            .clone()
            .unwrap_or_else(|| String::from_utf16_lossy(&d.path));
        let key = battery::key("hidpp-gate", &format!("{}:{:04x}", route, d.product_id));
        groups.entry(key).or_default().push(d.clone());
    }
    let mut devices = Vec::new();
    for (group, mut endpoints) in groups {
        endpoints.sort_by(|a, b| a.usage.cmp(&b.usage).then_with(|| a.path.cmp(&b.path)));
        if endpoints.len() > 2
            || endpoints
                .windows(2)
                .any(|pair| pair[0].usage == pair[1].usage)
        {
            continue;
        }
        let first = &endpoints[0];
        let receiver =
            openlogi_device_registry::receiver::find_receiver(first.vendor_id, first.product_id)
                .is_some();
        let slots: &[u8] = if receiver {
            &[1, 2, 3, 4, 5, 6]
        } else {
            &[0xff]
        };
        for &slot in slots {
            devices.push(Device {
                id: battery::key("hidpp-route", &format!("{group}:{slot}")),
                name: first.product.chars().take(64).collect(),
                container: first.container.clone(),
                kind: Kind::Other,
                group: group.clone(),
                slot,
                receiver,
                endpoints: endpoints.clone(),
            });
        }
    }
    devices
}

pub struct Observation {
    pub reading: Reading,
    pub identity: Evidence,
}
#[cfg(test)]
mod tests;
