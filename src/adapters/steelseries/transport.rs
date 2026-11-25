//! Native transport for the exact SteelSeries direct-percent allowlist.
use super::{parse, profile, request};
use crate::adapters::device::{Device, Handle, Reply};
use std::{
    mem::{size_of, zeroed},
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};
use windows_sys::Win32::Devices::HumanInterfaceDevice::*;

struct Descriptor(PHIDP_PREPARSED_DATA);
impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe {
            HidD_FreePreparsedData(self.0);
        }
    }
}

fn check(epoch: &AtomicU64, expected: u64, deadline: Instant) -> Result<(), String> {
    if epoch.load(Ordering::Acquire) != expected {
        Err("cancelled: SteelSeries epoch".into())
    } else if Instant::now() >= deadline {
        Err("timeout: SteelSeries transaction".into())
    } else {
        Ok(())
    }
}

pub(crate) fn query(
    device: &Device,
    epoch: &AtomicU64,
    expected: u64,
    deadline: Instant,
) -> Result<crate::battery::Reading, String> {
    check(epoch, expected, deadline)?;
    let profile = profile(device).ok_or("unsupported: SteelSeries collection")?;
    let check = || check(epoch, expected, deadline);
    check()?;
    let handle = Handle::open_status(device, "无法访问 SteelSeries 状态接口")?;
    let mut attributes: HIDD_ATTRIBUTES = unsafe { zeroed() };
    attributes.Size = size_of::<HIDD_ATTRIBUTES>() as u32;
    if !unsafe { HidD_GetAttributes(handle.0, &mut attributes) }
        || attributes.VendorID != device.vendor_id
        || attributes.ProductID != device.product_id
    {
        return Err("unsupported: SteelSeries endpoint identity changed".into());
    }
    let mut preparsed = 0;
    if !unsafe { HidD_GetPreparsedData(handle.0, &mut preparsed) } {
        return Err("unsupported: SteelSeries descriptor unavailable".into());
    }
    let descriptor = Descriptor(preparsed);
    let mut caps: HIDP_CAPS = unsafe { zeroed() };
    if unsafe { HidP_GetCaps(descriptor.0, &mut caps) } != HIDP_STATUS_SUCCESS
        || caps.UsagePage != device.usage_page
        || caps.Usage != device.usage
        || caps.InputReportByteLength != device.input_bytes
        || caps.OutputReportByteLength != device.output_bytes
    {
        return Err("unsupported: SteelSeries descriptor changed".into());
    }
    let mut input = vec![0; usize::from(caps.InputReportByteLength)];
    let mut output = vec![0; usize::from(caps.OutputReportByteLength)];
    // Verify report ID zero exists in this exact TLC without reading live data.
    for (kind, buffer) in [(HidP_Input, &mut input), (HidP_Output, &mut output)] {
        if unsafe {
            HidP_InitializeReportForID(
                kind,
                0,
                descriptor.0,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
            )
        } != HIDP_STATUS_SUCCESS
        {
            return Err("unsupported: SteelSeries report ID zero not declared".into());
        }
    }
    check()?;
    handle.flush("清理过期 SteelSeries 状态失败")?;
    let output = request(profile, output.len())?;
    let output_len = output.len();
    let (mut read, written) = Reply::request(&handle, input, output, deadline, check)?;
    if written != output_len {
        return Err("invalid_reply: incomplete SteelSeries output".into());
    }
    // Bound unrelated vendor status traffic as well as elapsed time.
    for _ in 0..8 {
        let bytes = read.next_frame()?;
        if bytes.is_empty() {
            return Err("disconnected: SteelSeries status closed".into());
        }
        if let Some(reading) = parse(profile, bytes)? {
            return Ok(reading);
        }
    }
    Err("invalid_reply: SteelSeries status frame budget".into())
}
