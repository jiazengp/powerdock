mod overlapped;

pub(crate) fn matches_interface_path(path: &[u16], changed: &[String]) -> bool {
    if changed.is_empty() || path.is_empty() {
        return false;
    }
    let end = path
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(path.len());
    let path = String::from_utf16_lossy(&path[..end]);
    changed
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(&path))
}
pub(super) use overlapped::Reply;
pub(super) const QUERY_TIMEOUT: &str = "电量查询超时";
use serde::Serialize;
use std::{
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Devices::{DeviceAndDriverInstallation::*, HumanInterfaceDevice::*},
    Foundation::*,
    Storage::FileSystem::*,
    System::{IO::*, Threading::*},
};

pub(super) struct Handle(pub(super) HANDLE);
impl Handle {
    pub(super) fn open_status(device: &Device, context: &str) -> Result<Self, String> {
        let raw = unsafe {
            CreateFileW(
                device.path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                null_mut(),
            )
        };
        if raw == INVALID_HANDLE_VALUE {
            return Err(error(context));
        }
        Ok(Self(raw))
    }

    pub(super) fn flush(&self, context: &str) -> Result<(), String> {
        if unsafe { HidD_FlushQueue(self.0) } {
            Ok(())
        } else {
            Err(error(context))
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
struct DeviceSet(HDEVINFO);
impl Drop for DeviceSet {
    fn drop(&mut self) {
        unsafe {
            SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}
fn error(context: &str) -> String {
    format!("{context}：{}", std::io::Error::last_os_error())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Device {
    #[serde(skip)]
    pub path: Vec<u16>,
    #[serde(skip)]
    pub container: Option<String>,
    pub vendor_id: u16,
    pub product_id: u16,
    pub product: String,
    pub usage_page: u16,
    pub usage: u16,
    pub input_bytes: u16,
    pub output_bytes: u16,
}
pub fn enumerate() -> Result<Vec<Device>, String> {
    unsafe {
        let mut guid = zeroed();
        HidD_GetHidGuid(&mut guid);
        let raw = SetupDiGetClassDevsW(
            &guid,
            null(),
            null_mut(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        );
        if raw == -1 {
            return Err(error("无法枚举 HID"));
        }
        let set = DeviceSet(raw);
        let mut result = Vec::new();
        for index in 0..4096 {
            let mut entry: SP_DEVICE_INTERFACE_DATA = zeroed();
            entry.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
            if SetupDiEnumDeviceInterfaces(set.0, null(), &guid, index, &mut entry) == 0 {
                if GetLastError() == ERROR_NO_MORE_ITEMS {
                    return Ok(result);
                }
                return Err(error("HID 枚举失败"));
            }
            let mut needed = 0;
            SetupDiGetDeviceInterfaceDetailW(set.0, &entry, null_mut(), 0, &mut needed, null_mut());
            if !(8..=65536).contains(&needed) {
                continue;
            }
            let mut storage = vec![0u64; (needed as usize).div_ceil(8)];
            let detail = storage
                .as_mut_ptr()
                .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
            (*detail).cbSize = if cfg!(target_pointer_width = "64") {
                8
            } else {
                6
            };
            let mut info: SP_DEVINFO_DATA = zeroed();
            info.cbSize = size_of::<SP_DEVINFO_DATA>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(
                set.0,
                &entry,
                detail,
                needed,
                &mut needed,
                &mut info,
            ) == 0
            {
                continue;
            }
            let ptr = storage.as_ptr().cast::<u8>().add(4).cast::<u16>();
            let raw_path = std::slice::from_raw_parts(ptr, (needed as usize - 4) / 2);
            let end = raw_path
                .iter()
                .position(|x| *x == 0)
                .ok_or("HID 路径没有终止符")?;
            let mut path = raw_path[..end].to_vec();
            path.push(0);
            let raw = CreateFileW(
                path.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                0,
                null_mut(),
            );
            if raw == INVALID_HANDLE_VALUE {
                continue;
            }
            let handle = Handle(raw);
            let mut attributes: HIDD_ATTRIBUTES = zeroed();
            attributes.Size = size_of::<HIDD_ATTRIBUTES>() as u32;
            if !HidD_GetAttributes(handle.0, &mut attributes) {
                continue;
            }
            let mut preparsed = 0;
            if !HidD_GetPreparsedData(handle.0, &mut preparsed) {
                continue;
            }
            let mut caps: HIDP_CAPS = zeroed();
            let status = HidP_GetCaps(preparsed, &mut caps);
            HidD_FreePreparsedData(preparsed);
            if status != 0x110000 {
                continue;
            }
            let mut product = [0u16; 256];
            let name = if HidD_GetProductString(handle.0, product.as_mut_ptr().cast(), 512) {
                String::from_utf16_lossy(
                    &product[..product
                        .iter()
                        .position(|x| *x == 0)
                        .unwrap_or(product.len())],
                )
            } else {
                String::new()
            };
            result.push(Device {
                path,
                container: super::system::container(info.DevInst),
                vendor_id: attributes.VendorID,
                product_id: attributes.ProductID,
                product: name,
                usage_page: caps.UsagePage,
                usage: caps.Usage,
                input_bytes: caps.InputReportByteLength,
                output_bytes: caps.OutputReportByteLength,
            });
        }
        Err("HID 接口数量超过限制".into())
    }
}
