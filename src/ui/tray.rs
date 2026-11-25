//! Tray rendering and shell registration.
use super::*;

impl App {
    pub(super) fn tray_data(&self) -> NOTIFYICONDATAW {
        unsafe {
            let mut data: NOTIFYICONDATAW = zeroed();
            data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            data.hWnd = self.host;
            data.uID = 1;
            data.guidItem = self.guid;
            data
        }
    }
    pub(super) fn update_tray(&mut self, add: bool) -> Result<(), String> {
        if self.headless {
            return Ok(());
        }
        unsafe {
            let taskbar = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
            if !taskbar.is_null() {
                self.dpi = GetDpiForWindow(taskbar).max(96);
            }
            let size = GetSystemMetricsForDpi(SM_CXSMICON, self.dpi).max(16);
            let number = self.number();
            let percent = self
                .last
                .as_ref()
                .filter(|_| {
                    self.devices
                        .iter()
                        .find(|d| Some(&d.id) == self.selected.as_ref())
                        .is_some_and(|d| d.reading(settings::now()).is_some())
                })
                .and_then(|(r, _)| r.value.icon_fill());
            let charging = self
                .last
                .as_ref()
                .is_some_and(|(r, _)| r.power == Power::Charging)
                && self.fresh();
            let style = icon::appearance(self.dark, high_contrast(), percent, charging);
            let key = format!(
                "{number}:{}:{}:{size}:{percent:?}:{:?}",
                style.foreground,
                style.fill,
                self.kind()
            );
            let new_icon = if add || key != self.icon_key {
                Some(icon::create(percent, style, size, self.kind())?)
            } else {
                None
            };
            let mut data = self.tray_data();
            data.uFlags = NIF_GUID | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
            data.uCallbackMessage = WM_TRAY;
            copy_wide(
                &mut data.szTip,
                &format!(
                    "{}{} · {}{} · {}",
                    if self.demo { "[模拟] " } else { "" },
                    self.device_name(),
                    number,
                    if self.last.as_ref().and_then(|(r, _)| r.percent()).is_some() && number != "--"
                    {
                        "%"
                    } else {
                        ""
                    },
                    i18n::status(&self.status())
                ),
            );
            if let Some(icon) = new_icon {
                data.uFlags |= NIF_ICON;
                data.hIcon = icon;
            }
            if Shell_NotifyIconW(if add { NIM_ADD } else { NIM_MODIFY }, &data) == 0 {
                if let Some(icon) = new_icon {
                    DestroyIcon(icon);
                }
                return Err(format!(
                    "无法更新托盘图标：{}",
                    std::io::Error::last_os_error()
                ));
            }
            if let Some(icon) = new_icon {
                if !self.icon.is_null() {
                    DestroyIcon(self.icon);
                }
                self.icon = icon;
                self.icon_key = key;
            }
            if add {
                data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                Shell_NotifyIconW(NIM_SETVERSION, &data);
            }
            Ok(())
        }
    }
}
