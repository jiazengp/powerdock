//! Native descriptor metadata only; never reads input or feature reports.
use super::*;
use crate::adapters::device;

pub(super) struct Metadata {
    handle: HANDLE,
    pub(super) preparsed: PHIDP_PREPARSED_DATA,
    caps: HIDP_CAPS,
}
impl Drop for Metadata {
    fn drop(&mut self) {
        unsafe {
            HidD_FreePreparsedData(self.preparsed);
            CloseHandle(self.handle);
        }
    }
}
impl Metadata {
    pub(super) fn open(path: &[u16]) -> Result<Self, String> {
        unsafe {
            let handle = CreateFileW(
                path.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                0,
                null_mut(),
            );
            if handle == INVALID_HANDLE_VALUE {
                return Err(match GetLastError() {
                    ERROR_ACCESS_DENIED => "access_denied: HID 集合元数据访问被拒绝",
                    ERROR_SHARING_VIOLATION => "busy: HID 集合被独占",
                    ERROR_FILE_NOT_FOUND | ERROR_DEVICE_NOT_CONNECTED => {
                        "disconnected: HID 集合已断开"
                    }
                    _ => "unsupported: HID 集合元数据无法访问",
                }
                .into());
            }
            let mut preparsed = 0;
            if !HidD_GetPreparsedData(handle, &mut preparsed) {
                CloseHandle(handle);
                return Err("unsupported: HID 描述符不可用".into());
            }
            let mut metadata = Self {
                handle,
                preparsed,
                caps: zeroed(),
            };
            if HidP_GetCaps(preparsed, &mut metadata.caps) != HIDP_STATUS_SUCCESS {
                return Err("unsupported: HID 集合能力无法解析".into());
            }
            Ok(metadata)
        }
    }

    pub(super) fn fields(&self) -> Result<Vec<Field>, String> {
        let mut result = Vec::new();
        for (report_type, mut count, bytes) in [
            (
                ReportType::Feature,
                self.caps.NumberFeatureValueCaps,
                self.caps.FeatureReportByteLength,
            ),
            (
                ReportType::Input,
                self.caps.NumberInputValueCaps,
                self.caps.InputReportByteLength,
            ),
        ] {
            if count == 0 {
                continue;
            }
            if count > MAX_CAPS {
                return Err("unsupported: HID 描述符字段数量超过限制".into());
            }
            let mut caps = vec![HIDP_VALUE_CAPS::default(); usize::from(count)];
            let status = unsafe {
                HidP_GetValueCaps(
                    report_type.native(),
                    caps.as_mut_ptr(),
                    &mut count,
                    self.preparsed,
                )
            };
            if status != HIDP_STATUS_SUCCESS || usize::from(count) > caps.len() {
                return Err("unsupported: HID 电量字段描述符不可用".into());
            }
            for cap in &caps[..usize::from(count)] {
                if let Some(field) = Field::from_cap(cap, report_type, bytes)
                    && !result.contains(&field)
                {
                    result.push(field);
                }
                if result.len() > MAX_FIELDS {
                    return Err("unsupported: HID 电池字段数量超过限制".into());
                }
            }
        }
        Ok(result)
    }
}

/// Inspect one descriptor without querying battery reports. Discovery and the
/// compatibility inventory share exactly the same field qualification.
pub(crate) fn inspect(native: &device::Device) -> Result<Option<Device>, String> {
    let metadata = Metadata::open(&native.path)?;
    let fields = metadata.fields()?;
    if fields.is_empty() {
        return Ok(None);
    }
    let kind = match (metadata.caps.UsagePage, metadata.caps.Usage) {
        (1, 2) => Kind::Mouse,
        (1, 6 | 7) => Kind::Keyboard,
        (1, 4 | 5 | 8) => Kind::Gamepad,
        _ => Kind::Other,
    };
    Ok(Some(Device {
        id: battery::key("hid-standard", &String::from_utf16_lossy(&native.path)),
        name: if native.product.is_empty() {
            "HID 电池设备".into()
        } else {
            native.product.clone()
        },
        associated: false,
        container: native.container.clone(),
        kind,
        path: native.path.clone(),
        fields,
    }))
}

pub fn enumerate_from(interfaces: &[device::Device]) -> Result<Vec<Device>, String> {
    Ok(interfaces
        .iter()
        .filter_map(|native| inspect(native).ok().flatten())
        .collect())
}
