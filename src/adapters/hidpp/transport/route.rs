//! Accepted vendor collection/report shapes; metadata and parsed descriptors only.
use super::*;
use windows::Devices::HumanInterfaceDevice::HidReportType;
const MAX_DESCRIPTIONS: u32 = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Route {
    Short,
    Long,
}
impl Route {
    pub(super) fn id(self) -> u8 {
        match self {
            Self::Short => 0x10,
            Self::Long => 0x11,
        }
    }
    pub(super) fn len(self) -> usize {
        match self {
            Self::Short => 7,
            Self::Long => 20,
        }
    }
    pub(super) fn usage(self) -> u16 {
        match self {
            Self::Short => 1,
            Self::Long => 2,
        }
    }
    pub(super) fn from_metadata(device: &device::Device) -> Option<Self> {
        if device.vendor_id != 0x046d || device.usage_page != 0xff00 {
            return None;
        }
        let route = match device.usage {
            1 => Self::Short,
            2 => Self::Long,
            _ => return None,
        };
        (usize::from(device.input_bytes) == route.len()
            && usize::from(device.output_bytes) == route.len())
        .then_some(route)
    }
    pub(super) fn accepts(self, data: &[u8]) -> bool {
        data.len() == self.len() && data.first() == Some(&self.id())
    }
}

// Inspect parsed descriptor facts, never get a live input/feature report merely
// to discover capabilities. Both Input and Output must declare the exact ID.
pub(super) fn declares(
    device: &HidDevice,
    report_type: HidReportType,
    route: Route,
) -> Result<bool, String> {
    let boolean = device
        .GetBooleanControlDescriptions(report_type, 0xff00, route.usage())
        .map_err(win_error)?;
    let numeric = device
        .GetNumericControlDescriptions(report_type, 0xff00, route.usage())
        .map_err(win_error)?;
    let boolean_count = boolean.Size().map_err(win_error)?;
    let numeric_count = numeric.Size().map_err(win_error)?;
    if boolean_count > MAX_DESCRIPTIONS || numeric_count > MAX_DESCRIPTIONS {
        return Err("unsupported: HID++ descriptor exceeds field budget".into());
    }
    for index in 0..boolean_count {
        let description = boolean.GetAt(index).map_err(win_error)?;
        if description.ReportId().map_err(win_error)? == u16::from(route.id())
            && description.ReportType().map_err(win_error)? == report_type
            && description.UsagePage().map_err(win_error)? == 0xff00
            && description.UsageId().map_err(win_error)? == route.usage()
        {
            return Ok(true);
        }
    }
    for index in 0..numeric_count {
        let description = numeric.GetAt(index).map_err(win_error)?;
        if description.ReportId().map_err(win_error)? == u16::from(route.id())
            && description.UsagePage().map_err(win_error)? == 0xff00
            && description.UsageId().map_err(win_error)? == route.usage()
        {
            return Ok(true);
        }
    }
    Ok(false)
}
