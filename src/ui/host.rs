//! Native host lifecycle and window-message dispatch.
use super::*;
mod messages;
#[cfg(test)]
mod tests;
use messages::wndproc;

struct Context {
    app: UnsafeCell<App>,
    active: Cell<bool>,
    deferred: Cell<u32>,
    pending_interfaces: RefCell<Vec<String>>,
    host: Cell<HWND>,
    generation: Arc<AtomicU64>,
}
struct DispatchGuard<'a>(&'a Context);
impl Drop for DispatchGuard<'_> {
    fn drop(&mut self) {
        unsafe {
            self.0.active.set(false);
            let pending = self.0.deferred.replace(0);
            let host = self.0.host.get();
            if pending & 2 != 0 {
                PostMessageW(host, WM_RESCAN, 0, 0);
            }
            if pending & 1 != 0 {
                PostMessageW(host, WM_RESULT, 0, 0);
            }
            if pending & 4 != 0 {
                PostMessageW(host, WM_SETTINGCHANGE, 0, 0);
            }
        }
    }
}
pub fn run(args: &[String]) -> Result<(), String> {
    unsafe {
        let _com = notify::Com::initialize()?;
        let demo = args.iter().any(|x| x == "--preview");
        let host_title = if demo {
            "PowerDock Preview Host"
        } else {
            "PowerDock Host"
        };
        if args.iter().any(|arg| arg == "--quit") {
            let host = FindWindowW(wide("PowerDock.Host").as_ptr(), wide(host_title).as_ptr());
            if !host.is_null() && PostMessageW(host, WM_CLOSE, 0, 0) == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            return Ok(());
        }
        let mutex = CreateMutexW(
            null(),
            0,
            wide(if demo {
                "Local\\PowerDock.Native.Preview"
            } else {
                "Local\\PowerDock.Native"
            })
            .as_ptr(),
        );
        if mutex.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let host = FindWindowW(wide("PowerDock.Host").as_ptr(), wide(host_title).as_ptr());
            if !host.is_null() {
                PostMessageW(host, WM_OPEN, 0, 0);
            }
            CloseHandle(mutex);
            return Ok(());
        }
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = GetModuleHandleW(null());
        let class_name = wide("PowerDock.Host");
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            ..zeroed()
        };
        if RegisterClassW(&class) == 0 {
            CloseHandle(mutex);
            return Err(std::io::Error::last_os_error().to_string());
        }
        let (config, mut load_error) = if demo {
            (Settings::default(), None)
        } else {
            settings::load()
        };
        i18n::set_language(config.language);
        let directory = if demo {
            Some(crate::identity::Directory::default())
        } else {
            match crate::identity::Directory::load() {
                Ok(d) => Some(d),
                Err(e) => {
                    load_error = Some(format!("设备目录未加载：{e}"));
                    None
                }
            }
        };
        let controls = monitor::Controls::new(config.providers.clone());
        let update_controller = updates::Controller::new(demo, config.updates.preview);
        let mut identity = settings::identity(&updates::launcher_path());
        if demo {
            identity[15] ^= 0x80;
        }
        let guid = GUID {
            data1: u32::from_le_bytes(identity[..4].try_into().unwrap()),
            data2: u16::from_le_bytes(identity[4..6].try_into().unwrap()),
            data3: u16::from_le_bytes(identity[6..8].try_into().unwrap()),
            data4: identity[8..].try_into().unwrap(),
        };
        let generation = Arc::new(AtomicU64::new(1));
        let context = Box::new(Context {
            active: Cell::new(true),
            deferred: Cell::new(0),
            pending_interfaces: RefCell::new(Vec::new()),
            host: Cell::new(null_mut()),
            generation: generation.clone(),
            app: UnsafeCell::new(App {
                host: null_mut(),
                icon: null_mut(),
                icon_key: String::new(),
                guid,
                taskbar_message: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
                notification: null_mut(),
                config,
                last: None,
                devices: Vec::new(),
                catalog: Vec::new(),
                selected: None,
                error: None,
                notice_error: load_error,
                busy: false,
                dark: theme_dark(),
                demo,
                window_mode: args.iter().any(|arg| arg == "--window"),
                headless: demo && args.iter().any(|arg| arg == "--headless"),
                generation,
                latest: Arc::new(Mutex::new(None)),
                tx: None,
                worker: None,
                last_manual: None,
                dpi: 96,
                child: None,
                ui: null_mut(),
                page: 0,
                updates: update_controller,
                notice_queue: notify::OverlayQueue::default(),
                directory,
                blocked_reminders: Default::default(),
                endpoint_ids: std::collections::BTreeMap::new(),
                controls,
            }),
        });
        let pointer = (&*context as *const Context).cast();
        let app = &mut *context.app.get();
        let host = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            wide(host_title).as_ptr(),
            0,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            pointer,
        );
        if host.is_null() {
            CloseHandle(mutex);
            return Err(std::io::Error::last_os_error().to_string());
        }
        app.host = host;
        context.host.set(host);
        if let Err(e) = app.update_tray(true) {
            SetWindowLongPtrW(host, GWLP_USERDATA, 0);
            DestroyWindow(host);
            CloseHandle(mutex);
            return Err(e);
        }
        let mut filter: DEV_BROADCAST_DEVICEINTERFACE_W = zeroed();
        filter.dbcc_size = size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32;
        filter.dbcc_devicetype = DBT_DEVTYP_DEVICEINTERFACE;
        HidD_GetHidGuid(&mut filter.dbcc_classguid);
        app.notification = RegisterDeviceNotificationW(
            host,
            (&filter as *const DEV_BROADCAST_DEVICEINTERFACE_W).cast(),
            DEVICE_NOTIFY_WINDOW_HANDLE,
        );
        if demo {
            app.devices = battery::preview_devices();
            app.select_current();
            let _ = app.update_tray(false);
        } else {
            app.start_worker();
        }
        SetTimer(host, TIMER_STALE, 30_000, None);
        SetTimer(host, TIMER_UPDATE, 60_000, None);
        if let Some(index) = args.iter().position(|x| x == "--run-for")
            && let Some(s) = args
                .get(index + 1)
                .and_then(|x| x.parse::<u32>().ok())
                .filter(|v| *v > 0 && *v <= 3600)
        {
            SetTimer(host, TIMER_EXIT, s * 1000, None);
        }
        if !app.headless && (demo || args.iter().any(|x| x == "--show" || x == "--window")) {
            app.open_ui(0);
        }
        if !demo
            && startup_enabled()
            && let Err(e) = set_startup(true)
        {
            settings::log_error(&e);
        }
        drop(DispatchGuard(&context));
        let mut message: MSG = zeroed();
        loop {
            let result = GetMessageW(&mut message, null_mut(), 0, 0);
            if result <= 0 {
                if result < 0 {
                    settings::log_error(&std::io::Error::last_os_error().to_string());
                }
                break;
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        context.active.set(true);
        let app = &mut *context.app.get();
        app.stop_worker();
        app.close_ui();
        app.updates.cancel();
        if !demo
            && app.config.updates.auto_download
            && app.updates.status.state == "ready"
            && let Err(error) = app.updates.apply(false)
        {
            settings::log_error(&format!("Update installation failed: {error}"));
        }
        let data = app.tray_data();
        Shell_NotifyIconW(NIM_DELETE, &data);
        if !app.icon.is_null() {
            DestroyIcon(app.icon);
        }
        if !app.notification.is_null() {
            UnregisterDeviceNotification(app.notification);
        }
        if !demo {
            app.config.alerts.last_seen = settings::now();
            if let Err(e) = settings::save(&app.config) {
                settings::log_error(&e);
            }
        }
        SetWindowLongPtrW(host, GWLP_USERDATA, 0);
        DestroyWindow(host);
        CloseHandle(mutex);
        Ok(())
    }
}
