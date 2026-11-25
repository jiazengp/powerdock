//! Descriptor-driven HID Battery Strength (Generic Device Controls 06/20).
//!
//! Discovery opens metadata handles only. Acquisition uses WinRT-owned async
//! Feature reports: no Input subscription/read, writes, or guessed report IDs.
//! Native HidD_GetFeature / synchronous hidapi are deliberately not used: a
//! cancelled native OVERLAPPED cannot safely be freed before driver completion.
mod discovery;
mod field;
use discovery::Metadata;
pub use discovery::enumerate_from;
pub(crate) use discovery::inspect;
use field::{Field, ReportType};

use crate::battery::{self, Kind, Level, Power, Reading, Value};
use std::{
    fmt,
    mem::zeroed,
    ptr::{null, null_mut},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use windows::{
    Devices::HumanInterfaceDevice::HidDevice,
    Storage::{FileAccessMode, Streams::DataReader},
    core::{HSTRING, RuntimeType},
};
use windows_future::IAsyncOperation;
use windows_sys::Win32::{Devices::HumanInterfaceDevice::*, Foundation::*, Storage::FileSystem::*};

const PAGE: u16 = 0x06;
const USAGE: u16 = 0x20;
const MAX_CAPS: u16 = 512;
const MAX_FIELDS: usize = 16;
const MAX_REPORT: u16 = 1024;

#[derive(Clone, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub container: Option<String>,
    pub kind: Kind,
    pub associated: bool,
    path: Vec<u16>,
    fields: Vec<Field>,
}
impl fmt::Debug for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Opaque identity and descriptor facts are safe; never log the raw path.
        f.debug_struct("HidBatteryDevice")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("container", &self.container)
            .field("kind", &self.kind)
            .field("fields", &self.fields)
            .finish()
    }
}

struct OpenDevice(HidDevice);
impl Drop for OpenDevice {
    fn drop(&mut self) {
        let _ = self.0.Close();
    }
}
fn check_session(generation: &AtomicU64, expected: u64, deadline: Instant) -> Result<(), String> {
    if generation.load(Ordering::Acquire) != expected {
        Err("cancelled: HID 设备会话已变更".into())
    } else if Instant::now() >= deadline {
        Err("timeout: HID 电量查询超时".into())
    } else {
        Ok(())
    }
}
fn async_error(code: Option<i32>) -> String {
    match code.map(|code| code as u32) {
        Some(0x80070005) => "access_denied: HID Feature 集合访问被拒绝",
        Some(0x80070020) => "busy: HID Feature 集合被独占",
        Some(0x8007048f | 0x80070002) => "disconnected: HID Feature 集合已断开",
        _ => "unsupported: HID WinRT Feature 接口不可用、集合受限或读取失败",
    }
    .into()
}
fn wait<T: RuntimeType>(
    operation: windows::core::Result<IAsyncOperation<T>>,
    deadline: Instant,
    generation: &AtomicU64,
    expected: u64,
) -> Result<T, String> {
    let operation = operation.map_err(|error| async_error(Some(error.code().0)))?;
    // Keep the operation's HRESULT: the shared waiter intentionally emits only
    // generic transport text, which cannot distinguish denied access from loss.
    crate::winrt::wait(Ok(operation.clone()), deadline, generation, expected).map_err(|_| {
        check_session(generation, expected, deadline)
            .err()
            .unwrap_or_else(|| async_error(operation.ErrorCode().ok().map(|code| code.0)))
    })
}
impl Device {
    pub(crate) fn matches_interface(&self, changed: &[String]) -> bool {
        super::device::matches_interface_path(&self.path, changed)
    }

    #[cfg(test)]
    pub(crate) fn test_route(path: &str) -> Self {
        let mut device = Self::test_associated("synthetic", "hid-route");
        device.container = None;
        device.associated = false;
        device.path = path.encode_utf16().chain(Some(0)).collect();
        device
    }

    #[cfg(test)]
    pub(crate) fn test_associated(container: &str, route: &str) -> Self {
        Self {
            id: route.into(),
            name: container.into(),
            container: Some(container.into()),
            associated: true,
            kind: Kind::Mouse,
            path: Vec::new(),
            fields: vec![Field {
                report_type: ReportType::Feature,
                report_id: 0,
                report_bytes: 2,
                link: 0,
                bits: 8,
                min: 0,
                max: 100,
                has_null: false,
            }],
        }
    }
    pub fn feature_readable(&self) -> bool {
        self.fields
            .iter()
            .filter(|f| f.report_type == ReportType::Feature)
            .count()
            == 1
    }
    pub fn query(&self, generation: &AtomicU64, expected: u64) -> Result<Reading, String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        check_session(generation, expected, deadline)?;
        let features: Vec<_> = self
            .fields
            .iter()
            .filter(|f| f.report_type == ReportType::Feature)
            .collect();
        // Multiple Battery Strength fields may describe different batteries.
        // Leave component identity to a future reader rather than collapse them.
        if features.len() != 1 {
            return Err(if features.is_empty() {
                "unsupported: HID 电量仅在 Input 报告中，未捕获标准输入"
            } else {
                "unsupported: HID 多电池字段尚无可靠组件映射"
            }
            .into());
        }
        let field = features[0];
        let metadata = Metadata::open(&self.path)?;
        if metadata.fields()? != self.fields {
            return Err("cancelled: HID 描述符已变化，等待重新发现".into());
        }
        let end = self
            .path
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(self.path.len());
        let path = HSTRING::from_wide(&self.path[..end]);
        // WinRT owns each asynchronous report buffer, including after Cancel or
        // a late result. Never substitute native stack-backed OVERLAPPED I/O.
        let opened = wait(
            HidDevice::FromIdAsync(&path, FileAccessMode::Read),
            deadline,
            generation,
            expected,
        )?;
        let device = OpenDevice(opened);
        check_session(generation, expected, deadline)?;
        let report = wait(
            device
                .0
                .GetFeatureReportByIdAsync(u16::from(field.report_id)),
            deadline,
            generation,
            expected,
        )?;
        check_session(generation, expected, deadline)?;
        if report.Id().ok() != Some(u16::from(field.report_id)) {
            return Err("invalid_reply: HID Feature 报告 ID 不匹配".into());
        }
        let buffer = report
            .Data()
            .map_err(|_| "invalid_reply: HID Feature 数据不可用")?;
        if buffer.Length().ok() != Some(u32::from(field.report_bytes)) {
            return Err("invalid_reply: HID Feature 报告长度无效".into());
        }
        let reader =
            DataReader::FromBuffer(&buffer).map_err(|_| "invalid_reply: HID Feature 缓冲不可读")?;
        let mut bytes = vec![0; usize::from(field.report_bytes)];
        reader
            .ReadBytes(&mut bytes)
            .map_err(|_| "invalid_reply: HID Feature 报告截断")?;
        let value = field.decode(metadata.preparsed, &bytes)?;
        check_session(generation, expected, deadline)?;
        Ok(Reading::new(value, self.kind, Power::Unknown))
    }
}

#[cfg(test)]
mod tests;
