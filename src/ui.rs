//! Application state and monitoring coordination for the native host.
mod commands;
mod host;
mod notifications;
#[cfg(test)]
#[path = "ui/tests.rs"]
mod observation_contract_tests;
mod snapshot;
mod startup;
mod tray;
mod window;

pub use host::run;
use startup::{set_startup, startup_enabled};

use crate::{
    agent,
    battery::{self, Kind, Power, Reading, Source, Tracked},
    i18n, icon,
    monitor::{self, Command, Snapshot},
    notify,
    settings::{self, Settings, wide},
    updates,
};
use std::{
    cell::{Cell, RefCell, UnsafeCell},
    mem::{size_of, zeroed},
    os::windows::process::CommandExt,
    process::{Child, Command as ProcessCommand},
    ptr::{null, null_mut},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::{
    Win32::{
        Devices::HumanInterfaceDevice::HidD_GetHidGuid,
        Foundation::*,
        System::{DataExchange::COPYDATASTRUCT, LibraryLoader::*, Registry::*, Threading::*},
        UI::{Accessibility::*, HiDpi::*, Shell::*, WindowsAndMessaging::*},
    },
    core::GUID,
};
const WM_RESULT: u32 = WM_APP + 1;
const WM_TRAY: u32 = WM_APP + 2;
const WM_RESCAN: u32 = WM_APP + 5;
const WM_OPEN: u32 = WM_APP + 6;
const WM_UI_REGISTER: u32 = WM_APP + 7;
const WM_UI_CLOSED: u32 = WM_APP + 8;
const WM_UI_PAGE: u32 = WM_APP + 9;
const REFRESH: u16 = 101;
const EXIT: u16 = 103;
const STARTUP: u16 = 104;
const TEST_NOTICE: u16 = 105;
const SETTINGS: u16 = 102;
const CONFIG_THRESHOLD: u16 = 200;
const CONFIG_INTERVAL: u16 = 300;
const TIMER_STALE: usize = 1;
const TIMER_EXIT: usize = 2;
const TIMER_UPDATE: usize = 3;
const TIMER_TOPOLOGY: usize = 4;
const TIMER_NOTICE: usize = 5;
struct App {
    host: HWND,
    icon: HICON,
    icon_key: String,
    guid: GUID,
    taskbar_message: u32,
    notification: HDEVNOTIFY,
    config: Settings,
    last: Option<(Reading, u64)>,
    devices: Vec<Tracked>,
    catalog: Vec<monitor::Candidate>,
    selected: Option<String>,
    error: Option<String>,
    notice_error: Option<String>,
    busy: bool,
    dark: bool,
    demo: bool,
    window_mode: bool,
    headless: bool,
    generation: Arc<AtomicU64>,
    latest: Arc<Mutex<Option<Snapshot>>>,
    tx: Option<SyncSender<Command>>,
    worker: Option<thread::JoinHandle<()>>,
    last_manual: Option<Instant>,
    dpi: u32,
    child: Option<Child>,
    ui: HWND,
    page: u8,
    updates: updates::Controller,
    notice_queue: notify::OverlayQueue,
    directory: Option<crate::identity::Directory>,
    blocked_reminders: std::collections::BTreeSet<String>,
    endpoint_ids: std::collections::BTreeMap<String, Vec<String>>,
    controls: monitor::Controls,
}
pub fn error_box(error: &str) {
    unsafe {
        MessageBoxW(
            null_mut(),
            wide(error).as_ptr(),
            wide("PowerDock").as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}
fn copy_wide<const N: usize>(target: &mut [u16; N], value: &str) {
    target.fill(0);
    for (to, from) in target.iter_mut().take(N - 1).zip(value.encode_utf16()) {
        *to = from;
    }
}
fn theme_dark() -> bool {
    unsafe {
        let mut value = 1u32;
        let mut size = 4;
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize").as_ptr(),
            wide("SystemUsesLightTheme").as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&mut value as *mut u32).cast(),
            &mut size,
        );
        value == 0
    }
}
fn high_contrast() -> bool {
    unsafe {
        let mut info: HIGHCONTRASTW = zeroed();
        info.cbSize = size_of::<HIGHCONTRASTW>() as u32;
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            info.cbSize,
            (&mut info as *mut HIGHCONTRASTW).cast(),
            0,
        );
        info.dwFlags & HCF_HIGHCONTRASTON != 0
    }
}
impl App {
    fn start_worker(&mut self) {
        let (tx, rx) = mpsc::sync_channel(4);
        self.tx = Some(tx.clone());
        let generation = self.generation.clone();
        let latest = self.latest.clone();
        let host = self.host as usize;
        self.controls.set_interval(self.config.interval_secs);
        self.worker = Some(monitor::spawn(
            rx,
            latest,
            generation,
            (host, WM_RESULT),
            self.controls.clone(),
            tx,
        ));
    }

    fn stop_worker(&mut self) {
        self.generation.store(u64::MAX, Ordering::Release);
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Command::Stop);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
    fn refresh(&mut self, invalidate: bool) {
        if self.demo {
            return;
        }
        if self
            .last_manual
            .is_some_and(|time| time.elapsed() < Duration::from_secs(2))
            && !invalidate
        {
            return;
        }
        self.last_manual = Some(Instant::now());
        self.busy = true;
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(Command::Refresh { invalidate });
        }
        self.publish_state();
    }
    fn refresh_device(&mut self, id: &str) -> bool {
        crate::diagnostics::record("device_refresh_requested", serde_json::json!({"id":id}));
        if self.demo
            || !self.devices.iter().any(|d| d.id == id)
            || self
                .last_manual
                .is_some_and(|time| time.elapsed() < Duration::from_secs(2))
        {
            return false;
        }
        if self.tx.as_ref().is_some_and(|tx| {
            tx.try_send(Command::RefreshDevice {
                ids: self
                    .endpoint_ids
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| vec![id.into()]),
            })
            .is_ok()
        }) {
            self.last_manual = Some(Instant::now());
            return true;
        }
        false
    }
    fn consume_result(&mut self) {
        let snapshot = self.latest.lock().ok().and_then(|slot| slot.clone());
        if let Some(mut snapshot) = snapshot {
            if snapshot.generation != self.generation.load(Ordering::Acquire) {
                return;
            }
            for sample in &mut snapshot.devices {
                if sample.session != 0
                    && !self
                        .controls
                        .accepts(&sample.id, sample.session, sample.reader_epoch)
                {
                    sample.result = Err("读取器配置已变更，等待新读数".into());
                }
                if !self.config.providers.source(sample.source) {
                    sample.result = Err("电量读取器已关闭".into());
                }
            }
            let identities = crate::identity::reconcile(
                &mut self.directory,
                &mut self.config,
                &mut snapshot.devices,
                crate::identity::Directory::save,
                settings::save,
            );
            self.endpoint_ids = identities.endpoints;
            self.blocked_reminders = identities.blocked;
            if let Some(error) = identities.error {
                self.notice_error = Some(error);
            }
            // Strong aliases can collapse multiple endpoints. Prefer successful fresh
            // observations, retaining only one physical card before tracking state.
            battery::retain_preferred_samples(&mut snapshot.devices, settings::now());
            self.catalog = snapshot.catalog;
            self.busy = snapshot
                .devices
                .iter()
                .any(|d| d.result.as_ref().is_err_and(|e| e == "正在查询电量…"));
            let previous = std::mem::take(&mut self.devices);
            self.devices = snapshot
                .devices
                .into_iter()
                .map(|sample| {
                    let old = previous.iter().find(|d| d.id == sample.id);
                    Tracked::update(sample, old)
                })
                .collect();
            self.prune_identity_notices(&previous);
            for device in &self.devices {
                let summarize = |device: &Tracked| serde_json::json!({"id":device.id,"source":device.source,"connected":device.connection,"reading":device.last.as_ref().map(|(reading,_)|reading),"error":device.error});
                let current = summarize(device);
                let old = previous
                    .iter()
                    .find(|old| old.id == device.id)
                    .map(summarize);
                if old.as_ref() != Some(&current) {
                    crate::diagnostics::record(
                        "device_state_changed",
                        serde_json::json!({"before":old,"after":current}),
                    );
                }
            }
            for device in previous
                .iter()
                .filter(|old| !self.devices.iter().any(|new| new.id == old.id))
            {
                crate::diagnostics::record("device_removed", serde_json::json!({"id":device.id}));
            }
            self.devices.sort_by(|a, b| a.id.cmp(&b.id));
            self.select_current();
            if self.devices.is_empty() {
                self.error = Some(snapshot.error.unwrap_or_else(|| {
                    if self.busy {
                        "正在查询系统外设电量…".into()
                    } else {
                        "未找到可读取电量的外设，请检查连接与读取器设置".into()
                    }
                }));
            }
            self.try_replay_notices();
            self.maybe_alert();
            if let Err(e) = self.update_tray(false) {
                settings::log_error(&e);
            }
            self.publish_state();
        }
    }
    fn select_current(&mut self) {
        self.selected = battery::lowest(&self.devices, self.selected.as_deref(), settings::now())
            .or_else(|| {
                battery::lowest_level(&self.devices, self.selected.as_deref(), settings::now())
            });
        let current = self
            .devices
            .iter()
            .find(|d| Some(&d.id) == self.selected.as_ref());
        self.last = current.and_then(|d| d.last.clone());
        self.error = current.and_then(|d| d.error.clone());
        if current.is_none() {
            self.error = Some("没有可比较的有效电量：未知、离线和过期读数已排除".into());
        }
    }
    fn kind(&self) -> Kind {
        self.last.as_ref().map_or(Kind::Other, |(r, _)| r.kind)
    }
    fn device_name(&self) -> &str {
        self.devices
            .iter()
            .find(|d| Some(&d.id) == self.selected.as_ref())
            .map_or(
                if self.demo {
                    "无线鼠标"
                } else {
                    "PowerDock"
                },
                |d| d.name.as_str(),
            )
    }
    fn source(&self) -> Source {
        self.devices
            .iter()
            .find(|d| Some(&d.id) == self.selected.as_ref())
            .map_or(Source::Unknown, |d| d.source)
    }
    fn fresh(&self) -> bool {
        self.devices
            .iter()
            .find(|d| Some(&d.id) == self.selected.as_ref())
            .is_some_and(|d| {
                d.reading(settings::now())
                    .and_then(Reading::percent)
                    .is_some()
            })
    }
    fn number(&self) -> String {
        self.devices
            .iter()
            .find(|d| Some(&d.id) == self.selected.as_ref())
            .and_then(|d| d.reading(settings::now()))
            .map_or_else(
                || "--".into(),
                |r| {
                    r.percent()
                        .map_or_else(|| r.value.display(), |p| p.to_string())
                },
            )
    }
    fn status(&self) -> String {
        if self.demo {
            return "预览模式 · 模拟数据".into();
        }
        if let Some(error) = &self.error {
            return error.clone();
        }
        if let Some((r, _)) = &self.last {
            if !self
                .devices
                .iter()
                .find(|d| Some(&d.id) == self.selected.as_ref())
                .is_some_and(|d| d.reading(settings::now()).is_some())
            {
                return "电量读数已过期，请刷新".into();
            }
            return format!("{} · {}", r.state(), self.source().label());
        }
        "正在查询鼠标电量…".into()
    }
    fn save(&mut self) {
        if !self.demo
            && let Err(e) = settings::save(&self.config)
        {
            self.notice_error = Some(format!("设置未能保存：{e}"));
            settings::log_error(&e);
        }
    }
    fn command(&mut self, id: u16) {
        crate::diagnostics::record("ui_command", serde_json::json!({"id":id}));
        unsafe {
            match id {
                106 | 107 => {
                    if self.demo {
                        return;
                    }
                    let result = if id == 106 {
                        std::fs::create_dir_all(crate::diagnostics::root())
                            .map(|_| crate::diagnostics::root())
                            .map_err(|e| e.to_string())
                    } else {
                        crate::diagnostics::export(None, self.state_snapshot(false))
                            .map(|path| path.parent().unwrap_or(&path).to_owned())
                    };
                    match result {
                        Ok(path) => {
                            ShellExecuteW(
                                self.host,
                                settings::wide("open").as_ptr(),
                                settings::wide(&path).as_ptr(),
                                std::ptr::null(),
                                std::ptr::null(),
                                SW_SHOWNORMAL,
                            );
                        }
                        Err(error) => {
                            settings::log_error(&error);
                            self.notice_error = Some(error);
                            self.publish_state();
                        }
                    }
                }
                700 | 701 => {
                    let _ = self.agent_command(agent::Operation::UpdateCheck {
                        download: id == 701,
                    });
                }
                702 => {
                    let _ = self.agent_command(agent::Operation::UpdateApply {});
                }

                REFRESH => {
                    self.refresh(false);
                }
                SETTINGS => self.open_ui(1),
                EXIT => {
                    PostQuitMessage(0);
                }
                STARTUP => {
                    if let Err(e) = set_startup(!startup_enabled()) {
                        self.notice_error = Some(e.clone());
                        settings::log_error(&e);
                    }
                }
                TEST_NOTICE => {
                    let _ = self.agent_command(agent::Operation::NotificationTest {});
                }
                600..=602 | 610..=611 => {
                    let sort_by = match id {
                        600 => Some(settings::SortBy::Battery),
                        601 => Some(settings::SortBy::Type),
                        602 => Some(settings::SortBy::Charging),
                        _ => None,
                    };
                    let sort_order = match id {
                        610 => Some(settings::SortOrder::Asc),
                        611 => Some(settings::SortOrder::Desc),
                        _ => None,
                    };
                    let result = self.agent_command(agent::Operation::SettingsSet {
                        threshold: None,
                        interval_secs: None,
                        startup: None,
                        sort_by,
                        sort_order,
                    });
                    if result["ok"] != true {
                        self.notice_error = Some("排序设置未保存，请重试".into());
                    }
                }
                CONFIG_THRESHOLD..CONFIG_INTERVAL => {
                    let threshold = (id - CONFIG_THRESHOLD) as u8;
                    if settings::ALERT_THRESHOLDS.contains(&threshold) {
                        self.set_monitor_setting(Some(threshold), None);
                    }
                }
                CONFIG_INTERVAL..=420 => {
                    let interval = u64::from(id - CONFIG_INTERVAL);
                    if settings::POLL_INTERVALS.contains(&interval) {
                        self.set_monitor_setting(None, Some(interval));
                    }
                }
                _ => {}
            }
            self.publish_state();
        }
    }
    fn set_monitor_setting(&mut self, threshold: Option<u8>, interval_secs: Option<u64>) {
        let result = self.agent_command(agent::Operation::SettingsSet {
            threshold,
            interval_secs,
            startup: None,
            sort_by: None,
            sort_order: None,
        });
        if let Some(error) = result["error"]["message"].as_str() {
            self.notice_error = Some(error.into());
        }
    }
}
