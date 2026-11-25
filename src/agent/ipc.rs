//! Same-executable Windows message transport with bounded waits and payloads.
use super::{MAX_BYTES, MAX_SNAPSHOT_BYTES, Operation, Request, failure};
use crate::settings::wide;
use serde_json::Value;
use std::{
    cell::RefCell,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    System::{DataExchange::COPYDATASTRUCT, LibraryLoader::GetModuleHandleW, Threading::*},
    UI::WindowsAndMessaging::*,
};

const REPLY_TAG: usize = 0x4D424102;
const REQUEST_TAG: usize = 0x4D424101;
const SNAPSHOT_REQUEST_TAG: usize = 0x4D424103;
const TIMEOUT_MS: u32 = 1500;

pub fn is_request_tag(tag: usize) -> bool {
    matches!(tag, REQUEST_TAG | SNAPSHOT_REQUEST_TAG)
}

pub fn reply_limit(tag: usize, operation: &Operation) -> usize {
    if tag == SNAPSHOT_REQUEST_TAG
        && matches!(operation, Operation::Status {} | Operation::Devices { .. })
    {
        MAX_SNAPSHOT_BYTES
    } else {
        MAX_BYTES
    }
}

/// An HWND/PID is not enough: both peers must be processes from this installed executable.
pub fn same_executable(window: HWND) -> bool {
    unsafe {
        let mut pid = 0;
        if window.is_null() || GetWindowThreadProcessId(window, &mut pid) == 0 {
            return false;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut path = vec![0u16; 32768];
        let mut length = path.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length) != 0;
        CloseHandle(process);
        ok && std::env::current_exe().is_ok_and(|own| {
            own.to_string_lossy()
                .eq_ignore_ascii_case(&String::from_utf16_lossy(&path[..length as usize]))
        })
    }
}
struct ReplySlot {
    host: HWND,
    limit: usize,
    value: RefCell<Option<Value>>,
}
unsafe extern "system" fn receive(window: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lp as *const CREATESTRUCTW);
            SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let slot = GetWindowLongPtrW(window, GWLP_USERDATA) as *const ReplySlot;
        if message == WM_COPYDATA && !slot.is_null() && wp as HWND == (*slot).host && lp != 0 {
            let data = &*(lp as *const COPYDATASTRUCT);
            if data.dwData == REPLY_TAG
                && data.cbData as usize <= (*slot).limit
                && data.cbData > 0
                && !data.lpData.is_null()
            {
                let bytes =
                    std::slice::from_raw_parts(data.lpData.cast::<u8>(), data.cbData as usize);
                if let Ok(value) = serde_json::from_slice::<Value>(bytes)
                    && value["schema_version"] == 1
                    && value["ok"].is_boolean()
                {
                    let _ = (*slot).value.replace(Some(value));
                    return 1;
                }
            }
            return 0;
        }
        DefWindowProcW(window, message, wp, lp)
    }
}
pub(crate) fn call(operation: Operation, preview: bool) -> (Value, i32) {
    call_with_snapshot_support(operation, preview, true)
}

#[cfg(test)]
pub(crate) fn legacy_call(operation: Operation, preview: bool) -> (Value, i32) {
    call_with_snapshot_support(operation, preview, false)
}

fn call_with_snapshot_support(
    operation: Operation,
    preview: bool,
    supports_large_reply: bool,
) -> (Value, i32) {
    unsafe {
        let host = FindWindowW(
            wide("PowerDock.Host").as_ptr(),
            wide(if preview {
                "PowerDock Preview Host"
            } else {
                "PowerDock Host"
            })
            .as_ptr(),
        );
        if host.is_null() {
            return (
                failure(
                    "not_running",
                    "Start powerdock.exe before using live commands",
                ),
                4,
            );
        }
        if !same_executable(host) {
            return (
                failure(
                    "executable_mismatch",
                    "Use the executable at the same path as the running host",
                ),
                5,
            );
        }
        let tag =
            if supports_large_reply && reply_limit(SNAPSHOT_REQUEST_TAG, &operation) > MAX_BYTES {
                SNAPSHOT_REQUEST_TAG
            } else {
                REQUEST_TAG
            };
        let slot = ReplySlot {
            host,
            limit: reply_limit(tag, &operation),
            value: RefCell::new(None),
        };
        let name = wide("PowerDock.AgentReply");
        let instance = GetModuleHandleW(null());
        let class = WNDCLASSW {
            lpfnWndProc: Some(receive),
            hInstance: instance,
            lpszClassName: name.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&class);
        let window = CreateWindowExW(
            0,
            name.as_ptr(),
            name.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            null_mut(),
            instance,
            (&slot as *const ReplySlot).cast(),
        );
        if window.is_null() {
            return (
                failure(
                    "transport_failure",
                    std::io::Error::last_os_error().to_string(),
                ),
                5,
            );
        }
        if matches!(operation, Operation::Show { .. }) {
            let mut pid = 0;
            GetWindowThreadProcessId(host, &mut pid);
            AllowSetForegroundWindow(pid);
        }
        let request = Request {
            version: 1,
            command: operation,
        };
        let bytes = serde_json::to_vec(&request).expect("fixed request types serialize");
        let mut data = COPYDATASTRUCT {
            dwData: tag,
            cbData: bytes.len() as u32,
            lpData: bytes.as_ptr().cast_mut().cast(),
        };
        // SMTO_BLOCK would deadlock the synchronous response. Only our bounded reply handler runs.
        let send = |data: &COPYDATASTRUCT| {
            let mut result = 0;
            SendMessageTimeoutW(
                host,
                WM_COPYDATA,
                window as usize,
                (data as *const COPYDATASTRUCT) as isize,
                SMTO_ABORTIFHUNG,
                TIMEOUT_MS,
                &mut result,
            )
        };
        let mut sent = send(&data);
        if tag == SNAPSHOT_REQUEST_TAG && sent != 0 && slot.value.borrow().is_none() {
            // An older host can ignore the new request tag. Both operations are read-only.
            data.dwData = REQUEST_TAG;
            sent = send(&data);
        }
        DestroyWindow(window);
        UnregisterClassW(name.as_ptr(), instance);
        if let Some(value) = slot.value.into_inner() {
            let code = if value["ok"] == true { 0 } else { 3 };
            return (value, code);
        }
        let message = if sent == 0 {
            "The host did not answer within the transport timeout"
        } else {
            "The host rejected this protocol version or was busy; retry once"
        };
        (failure("transport_failure", message), 5)
    }
}
pub fn send_reply(client: HWND, host: HWND, value: Value, limit: usize) -> bool {
    let mut bytes = value.to_string();
    if bytes.len() > limit {
        bytes = failure(
            "response_too_large",
            "The response exceeded the local IPC limit. For device snapshots, use devices list --page-size 8",
        )
        .to_string();
    }
    let data = COPYDATASTRUCT {
        dwData: REPLY_TAG,
        cbData: bytes.len() as u32,
        lpData: bytes.as_ptr().cast_mut().cast(),
    };
    let mut result = 0;
    unsafe {
        SendMessageTimeoutW(
            client,
            WM_COPYDATA,
            host as usize,
            (&data as *const COPYDATASTRUCT) as isize,
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            200,
            &mut result,
        ) != 0
            && result != 0
    }
}
