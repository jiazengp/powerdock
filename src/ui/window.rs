//! Frontend process ownership, identity checks and bounded state delivery.
use super::*;

// Keep each message within the existing limit. The WinUI receiver bounds the
// assembled snapshot to the same size (see DevicePresentation.cpp).
const UI_SNAPSHOT_BYTES: usize = 8 * agent::MAX_BYTES;
const STATE_SINGLE: usize = 1;
const STATE_START: usize = 5;
const STATE_MORE: usize = 6;
const STATE_END: usize = 7;

impl App {
    pub(super) fn valid_ui(&self, window: HWND) -> bool {
        unsafe {
            if window.is_null() || IsWindow(window) == 0 {
                return false;
            }
            let mut pid = 0;
            GetWindowThreadProcessId(window, &mut pid);
            self.child.as_ref().is_some_and(|child| child.id() == pid)
        }
    }
    pub(super) fn reap_ui(&mut self) {
        let result = self
            .child
            .as_mut()
            .and_then(|child| match child.try_wait() {
                Ok(Some(status)) => Some((child.id(), status)),
                _ => None,
            });
        if let Some((pid, status)) = result {
            crate::diagnostics::record(
                "window_process_exited",
                serde_json::json!({"child_pid":pid,"exit_code":status.code(),"success":status.success()}),
            );
            if !status.success() {
                settings::log_error(&format!("Native UI exited: {status}"));
            }
            self.child = None;
            self.ui = null_mut();
        }
    }
    pub(super) fn open_ui(&mut self, page: u8) {
        if self.headless {
            return;
        }
        unsafe {
            // Tray activation is a user action. Delegate foreground permission only
            // to our own UI process; never relax the global foreground-lock policy.
            if page != 2 || self.child.is_none() {
                SetForegroundWindow(self.host);
            }
            self.page = page;
            self.reap_ui();
            self.refresh(false);
            if self.child.is_some() {
                if self.valid_ui(self.ui) {
                    if let Some(child) = &self.child {
                        AllowSetForegroundWindow(child.id());
                    }
                    PostMessageW(self.ui, WM_UI_PAGE, usize::from(page), 0);
                    if page != 2 {
                        SetForegroundWindow(self.ui);
                    }
                }
                return;
            }
            let result = (|| {
                let exe = std::env::current_exe()
                    .map_err(|error| error.to_string())?
                    .parent()
                    .ok_or("无法确定程序目录")?
                    .join("ui/PowerDockUI.exe");
                if !exe.is_file() {
                    return Err(
                        "未找到 WinUI 界面组件，请使用完整便携包，或先执行 scripts/build-ui.ps1。"
                            .to_owned(),
                    );
                }
                let mut command = ProcessCommand::new(&exe);
                let identifier = NOTIFYICONIDENTIFIER {
                    cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
                    hWnd: self.host,
                    uID: 1,
                    guidItem: self.guid,
                };
                let mut anchor: RECT = zeroed();
                if Shell_NotifyIconGetRect(&identifier, &mut anchor) < 0 {
                    let mut point = zeroed();
                    GetCursorPos(&mut point);
                    anchor = RECT {
                        left: point.x,
                        top: point.y,
                        right: point.x + 1,
                        bottom: point.y + 1,
                    };
                }
                command.args([
                    "--host",
                    &(self.host as usize).to_string(),
                    "--parent",
                    &std::process::id().to_string(),
                    "--page",
                    match page {
                        0 => "battery",
                        2 => "tray",
                        _ => "settings",
                    },
                ]);
                command.args([
                    "--anchor",
                    &anchor.left.to_string(),
                    &anchor.top.to_string(),
                    &anchor.right.to_string(),
                    &anchor.bottom.to_string(),
                ]);
                if self.demo {
                    command.arg("--preview");
                    let arguments: Vec<_> = std::env::args().collect();
                    if let Some(index) = arguments.iter().position(|value| value == "--theme")
                        && let Some(theme) = arguments.get(index + 1)
                        && matches!(theme.as_str(), "light" | "dark")
                    {
                        command.args(["--theme", theme.as_str()]);
                    }
                }
                if self.window_mode {
                    command.arg("--keep-open");
                }
                crate::diagnostics::prepare_child(&mut command, self.demo);
                command
                    .creation_flags(CREATE_NO_WINDOW)
                    .spawn()
                    .map_err(|e| format!("无法打开原生界面：{e}"))
            })();
            match result {
                Ok(mut child) => {
                    crate::diagnostics::attach_child(&mut child, "window");
                    AllowSetForegroundWindow(child.id());
                    self.child = Some(child);
                }
                Err(error) => {
                    settings::log_error(&error);
                    error_box(&error);
                }
            }
        }
    }
    pub(super) fn close_ui(&mut self) {
        unsafe {
            if self.valid_ui(self.ui) {
                PostMessageW(self.ui, WM_CLOSE, 0, 0);
            }
        }
        if let Some(child) = self.child.as_mut() {
            let deadline = Instant::now() + Duration::from_secs(2);
            while child.try_wait().is_ok_and(|s| s.is_none()) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
            if child.try_wait().is_ok_and(|s| s.is_none()) {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        self.child = None;
        self.ui = null_mut();
    }
    pub(super) fn publish_state(&mut self) {
        if !self.valid_ui(self.ui) {
            return;
        }
        let state = self.state_snapshot(false).to_string();
        if state.len() > UI_SNAPSHOT_BYTES {
            settings::log_error("界面状态超过 IPC 大小限制");
            return;
        }
        let deadline = Instant::now() + Duration::from_millis(200);
        let chunks = state.as_bytes().chunks(agent::MAX_BYTES);
        let count = chunks.len();
        for (index, chunk) in chunks.enumerate() {
            let tag = match (index == 0, index + 1 == count) {
                (true, true) => STATE_SINGLE,
                (true, false) => STATE_START,
                (false, true) => STATE_END,
                (false, false) => STATE_MORE,
            };
            let timeout = deadline
                .saturating_duration_since(Instant::now())
                .as_millis();
            if timeout == 0 || !self.send_state_packet(tag, chunk, timeout as u32) {
                break;
            }
        }
    }

    fn send_state_packet(&self, tag: usize, bytes: &[u8], timeout: u32) -> bool {
        let data = COPYDATASTRUCT {
            dwData: tag,
            cbData: bytes.len() as u32,
            lpData: bytes.as_ptr().cast_mut().cast(),
        };
        let mut result = 0;
        unsafe {
            // A stalled UI must not hold up battery monitoring or tray interaction.
            SendMessageTimeoutW(
                self.ui,
                WM_COPYDATA,
                self.host as usize,
                (&data as *const COPYDATASTRUCT) as isize,
                SMTO_ABORTIFHUNG | SMTO_BLOCK,
                timeout,
                &mut result,
            ) != 0
                && result != 0
        }
    }
}
