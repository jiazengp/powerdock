//! Restricted direct-percent mouse protocol; experimental and read-only.
//! Profile/decoder evidence: SteelMouse d6a849bb, MIT (vendor/STEELMOUSE-MIT.txt).
use crate::{
    adapters::device,
    battery::{self, Kind, Power, Reading, Value},
};
mod transport;
pub(crate) use transport::query;

#[cfg(test)]
pub(crate) mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Profile {
    pub name: &'static str,
    pub command: u8,
    pub wired: bool,
    pub interface: u8,
    layout: Layout,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    Aerox,
    RivalPro,
}

pub fn model(product_id: u16) -> Option<Profile> {
    let (name, command) = match product_id {
        0x1890 => ("SteelSeries Aerox 3 Gen 2 Wireless", 0xd2),
        0x1892 => ("SteelSeries Aerox 3 Gen 2 Wireless (USB)", 0x92),
        0x1880 => ("SteelSeries Rival Pro", 0x92),
        _ => return None,
    };
    Some(Profile {
        name,
        command,
        wired: product_id == 0x1892,
        interface: if product_id == 0x1880 { 2 } else { 3 },
        layout: if product_id == 0x1880 {
            Layout::RivalPro
        } else {
            Layout::Aerox
        },
    })
}

pub fn profile(d: &device::Device) -> Option<Profile> {
    let profile = model(d.product_id)?;
    let end = d.path.iter().position(|u| *u == 0).unwrap_or(d.path.len());
    let path = String::from_utf16_lossy(&d.path[..end]).to_ascii_lowercase();
    // Exact model-specific USB interface from upstream. A collection number is
    // not an interface number. Generic desktop mouse/keyboard TLCs never qualify.
    let interface = path
        .split(['#', '&'])
        .filter_map(|part| part.strip_prefix("mi_"))
        .collect::<Vec<_>>();
    (d.vendor_id == 0x1038
        && d.path.last() == Some(&0)
        && end + 1 == d.path.len()
        && d.usage_page >= 0xff00
        && d.usage != 0
        && interface.len() == 1
        && interface[0] == format!("{:02x}", profile.interface)
        && matches!(d.input_bytes, 64 | 65)
        && matches!(d.output_bytes, 64 | 65))
    .then_some(profile)
}

pub fn id(d: &device::Device) -> String {
    battery::key("steelseries-route", &String::from_utf16_lossy(&d.path))
}
pub fn matches_interface(d: &device::Device, changed: &[String]) -> bool {
    device::matches_interface_path(&d.path, changed)
}

/// Metadata only. Multiple qualifying TLCs in one route are ambiguous, not a
/// reason to query each endpoint until one responds.
pub fn enumerate_from(interfaces: &[device::Device]) -> Vec<device::Device> {
    let mut groups = std::collections::BTreeMap::<String, Vec<device::Device>>::new();
    for d in interfaces.iter().filter(|d| profile(d).is_some()) {
        let key = d
            .container
            .as_deref()
            .map_or_else(|| id(d), |c| format!("{c}:{:04x}", d.product_id));
        groups.entry(key).or_default().push(d.clone());
    }
    groups
        .into_values()
        .filter_map(|mut group| (group.len() == 1).then(|| group.remove(0)))
        .collect()
}

pub fn request(profile: Profile, length: usize) -> Result<Vec<u8>, String> {
    if !matches!(profile.command, 0x92 | 0xd2) || !matches!(length, 64 | 65) {
        return Err("unsupported: SteelSeries output shape".into());
    }
    let mut request = vec![0; length];
    request[1] = profile.command; // report ID zero, then the fixed battery GET.
    Ok(request)
}

/// Native Windows report-ID-zero prefix, then the model-specific payload.
/// The selected PID decides the layout; payload zeros never select an offset.
pub fn parse(profile: Profile, frame: &[u8]) -> Result<Option<Reading>, String> {
    if !matches!(frame.len(), 64 | 65) {
        return Err("invalid_reply: SteelSeries report length".into());
    }
    if frame[0] != 0 || frame[1] != profile.command {
        return Ok(None);
    }
    let raw = match profile.layout {
        Layout::Aerox => frame[2],
        Layout::RivalPro if frame[2] == 0 => frame[3],
        Layout::RivalPro => return Err("invalid_reply: SteelSeries response layout".into()),
    };
    let percent = raw & 0x7f;
    // Upstream treats zero as sleep/empty, not a verified empty battery reading.
    if percent == 0 || percent > 100 {
        return Err("invalid_reply: SteelSeries battery unavailable".into());
    }
    // The charging flag is not hardware-verified here. Do not infer Battery or
    // Charging and do not qualify this experimental source for notifications.
    Ok(Some(Reading::new(
        Value::reported(percent),
        Kind::Mouse,
        Power::Unknown,
    )))
}
