use crate::{battery::Kind, i18n, icon, settings};
use windows::{
    Data::Xml::Dom::XmlDocument,
    UI::Notifications::{ToastNotification, ToastNotificationManager},
    Win32::{
        Storage::EnhancedStorage::PKEY_AppUserModel_ID,
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize, IPersistFile, StructuredStorage::PROPVARIANT,
        },
        UI::Shell::{IShellLinkW, PropertiesSystem::IPropertyStore, ShellLink},
    },
    core::{HSTRING, Interface, PCWSTR},
};

const APP_ID: &str = "PowerDock.Native";
pub const TEST_ICON: icon::DeviceIcon = icon::DeviceIcon {
    kind: Kind::Mouse,
    fill: Some(15),
    charging: false,
};

pub struct Com;
impl Com {
    pub fn initialize() -> Result<Self, String> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|e| e.to_string())?;
        }
        Ok(Self)
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

fn register() -> Result<(), String> {
    unsafe {
        let exe = crate::updates::launcher_path();
        let folder = std::env::var_os("APPDATA")
            .map(std::path::PathBuf::from)
            .ok_or("没有 APPDATA 路径")?
            .join("Microsoft/Windows/Start Menu/Programs");
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let target = folder.join("PowerDock.lnk");
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        link.SetPath(PCWSTR(settings::wide(&exe).as_ptr()))
            .map_err(|e| e.to_string())?;
        link.SetDescription(PCWSTR(
            settings::wide("PowerDock peripheral battery alerts").as_ptr(),
        ))
        .map_err(|e| e.to_string())?;
        let properties: IPropertyStore = link.cast().map_err(|e| e.to_string())?;
        properties
            .SetValue(&PKEY_AppUserModel_ID, &PROPVARIANT::from(APP_ID))
            .map_err(|e| e.to_string())?;
        properties.Commit().map_err(|e| e.to_string())?;
        let file: IPersistFile = link.cast().map_err(|e| e.to_string())?;
        file.Save(PCWSTR(settings::wide(&target).as_ptr()), true)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
pub fn show(title: &str, body: &str, tag: &str, silent: bool) -> Result<(), String> {
    register()?;
    let xml = XmlDocument::new().map_err(|e| e.to_string())?;
    let audio = if silent {
        "<audio silent=\"true\"/>"
    } else {
        ""
    };
    xml.LoadXml(&HSTRING::from(format!("<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual>{audio}</toast>",escape(title),escape(body)))).map_err(|e|e.to_string())?;
    let toast = ToastNotification::CreateToastNotification(&xml).map_err(|e| e.to_string())?;
    toast
        .SetTag(&HSTRING::from(tag))
        .map_err(|e| e.to_string())?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))
        .and_then(|notifier| notifier.Show(&toast))
        .map_err(|e| e.to_string())
}

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    os::windows::process::CommandExt,
    process::{Child, Command},
    time::Instant,
};
#[derive(Clone, Debug)]
pub struct Notice {
    key: String,
    title: String,
    body: String,
    icon: icon::DeviceIcon,
    device_type: String,
    sound: bool,
    test: bool,
}
impl Notice {
    pub fn new(
        key: impl Into<String>,
        title: impl Into<String>,
        body: impl Into<String>,
        icon: icon::DeviceIcon,
        sound: bool,
        test: bool,
    ) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            body: body.into(),
            icon,
            device_type: i18n::device_kind(icon.kind),
            sound,
            test,
        }
    }
}
#[derive(Clone, Copy)]
pub struct DeferredRefresh {
    pub requested_at: Instant,
    pub after_idle: bool,
}
#[derive(Default)]
pub struct OverlayQueue {
    pending: VecDeque<Notice>,
    active: Option<Child>,
    deferred: BTreeSet<String>,
    idle_deferred: BTreeSet<String>,
    pub verifying: BTreeMap<String, DeferredRefresh>,
    pub error: Option<String>,
}
impl OverlayQueue {
    pub fn rename_device(&mut self, old: &str, canonical: &str) {
        if old == canonical {
            return;
        }
        for notice in &mut self.pending {
            if notice.key == old {
                notice.key = canonical.into();
            }
        }
        let mut seen = BTreeSet::new();
        self.pending
            .retain(|notice| seen.insert(notice.key.clone()));
        for queue in [&mut self.deferred, &mut self.idle_deferred] {
            if queue.remove(old) {
                queue.insert(canonical.into());
            }
        }
        if let Some(refresh) = self.verifying.remove(old) {
            self.verifying
                .entry(canonical.into())
                .and_modify(|current| {
                    current.requested_at = current.requested_at.max(refresh.requested_at);
                    current.after_idle |= refresh.after_idle;
                })
                .or_insert(refresh);
        }
    }
    pub fn remove(&mut self, key: &str) {
        self.pending.retain(|notice| notice.key != key);
        self.deferred.remove(key);
        self.idle_deferred.remove(key);
        self.verifying.remove(key);
    }
    pub fn enqueue(&mut self, notice: Notice) {
        self.idle_deferred.remove(&notice.key);
        if let Some(existing) = self.pending.iter_mut().find(|old| old.key == notice.key) {
            *existing = notice;
            return;
        }
        if self.pending.len() >= crate::battery::MAX_DEVICES {
            self.pending.pop_front();
        }
        self.pending.push_back(notice);
    }
    pub fn defer(&mut self, key: &str) {
        if self.deferred.len() < crate::battery::MAX_DEVICES {
            self.deferred.insert(key.into());
        }
    }
    pub fn deferred_count(&self) -> usize {
        self.deferred.union(&self.idle_deferred).count() + self.verifying.len()
    }
    pub fn defer_while_idle(&mut self, key: String) {
        self.verifying.remove(&key);
        if self.idle_deferred.len() < crate::battery::MAX_DEVICES
            && self.idle_deferred.insert(key.clone())
        {
            crate::diagnostics::record(
                "notification_deferred_idle",
                serde_json::json!({"device_id": key}),
            );
            self.deferred.remove(&key);
        }
    }
    pub fn waiting_for_activity(&self) -> bool {
        !self.idle_deferred.is_empty()
    }
    pub fn take_idle_deferred(&mut self) -> BTreeSet<String> {
        std::mem::take(&mut self.idle_deferred)
    }
    pub fn awaiting_fresh_read(&self, key: &str) -> bool {
        self.idle_deferred.contains(key) || self.verifying.contains_key(key)
    }
    pub fn clear_deferred(&mut self) {
        self.deferred.clear();
        self.verifying.retain(|_, refresh| refresh.after_idle);
    }
    pub fn cancel(&mut self) {
        self.pending.clear();
        self.deferred.clear();
        self.idle_deferred.clear();
        self.verifying.clear();
        if let Some(mut child) = self.active.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    pub fn take_deferred(&mut self) -> Vec<String> {
        if !self.deferred.is_empty() && !session_idle() && accepts_notifications() {
            std::mem::take(&mut self.deferred).into_iter().collect()
        } else {
            Vec::new()
        }
    }
    pub fn busy(&self) -> bool {
        self.active.is_some() || !self.pending.is_empty()
    }
    pub fn pump(&mut self, host: usize, reduced: bool, prefs: &settings::NotificationPreferences) {
        if let Some(child) = &mut self.active {
            match child.try_wait() {
                Ok(None) => return,
                Ok(Some(status)) => {
                    crate::diagnostics::record(
                        "notification_process_exited",
                        serde_json::json!({"child_pid":child.id(),"exit_code":status.code(),"success":status.success()}),
                    );
                    if !status.success() {
                        self.error = Some(format!("Overlay exited: {status}"));
                    }
                    self.active = None;
                }
                Err(error) => {
                    self.error = Some(error.to_string());
                    self.active = None;
                }
            }
        }
        if self.pending.iter().any(|notice| !notice.test) && session_idle() {
            self.defer_pending_while_idle();
        }
        if let Some(notice) = self.pending.pop_front() {
            let result = (|| {
                let exe = std::env::current_exe()
                    .map_err(|e| e.to_string())?
                    .parent()
                    .ok_or("Missing application directory")?
                    .join("ui/PowerDockUI.exe");
                if !exe.is_file() {
                    return Err("Missing native notification renderer".into());
                }
                let mut command = Command::new(exe);
                command.args([
                    "--notification",
                    "--title",
                    &notice.title,
                    "--body",
                    &notice.body,
                    "--host",
                    &host.to_string(),
                    "--parent",
                    &std::process::id().to_string(),
                ]);
                command.args([
                    "--position",
                    prefs.position.argument(),
                    "--duration",
                    &prefs.duration_secs.to_string(),
                    "--transparency",
                    &prefs.transparency_percent.to_string(),
                ]);
                if let Ok(encoded) = icon::notification::encode(notice.icon) {
                    command.args([
                        "--device-icon",
                        &encoded,
                        "--device-type",
                        &notice.device_type,
                    ]);
                }
                if prefs.monitor == settings::OverlayMonitor::Primary {
                    command.arg("--primary-monitor");
                }
                if reduced {
                    command.arg("--reduce-motion");
                }
                if notice.sound {
                    command.arg("--sound");
                }
                crate::diagnostics::prepare_child(&mut command, false);
                command
                    .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
                    .spawn()
                    .map_err(|e| e.to_string())
            })();
            match result {
                Ok(mut child) => {
                    crate::diagnostics::attach_child(&mut child, "notification");
                    self.active = Some(child);
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
    }
    fn defer_pending_while_idle(&mut self) {
        // A queued notification can reach the front after the user has stepped away.
        for notice in std::mem::take(&mut self.pending) {
            if notice.test {
                self.pending.push_back(notice);
            } else {
                self.defer_while_idle(notice.key);
            }
        }
    }
}
impl Drop for OverlayQueue {
    fn drop(&mut self) {
        if let Some(child) = &mut self.active {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
pub fn eligible_deferred(device: &crate::battery::Tracked, threshold: u8, now: u64) -> bool {
    device.visible(now)
        && device.source != crate::battery::Source::Windows
        && device.error.is_none()
        && threshold > 0
        && device.reading(now).is_some_and(|reading| {
            reading.discharging() && reading.percent().is_some_and(|p| p <= threshold)
        })
}
pub fn verified_deferred_reading(
    device: &crate::battery::Tracked,
    requested_at: Instant,
    now: u64,
) -> Option<&crate::battery::Reading> {
    (device.source != crate::battery::Source::Windows && device.value_clock >= requested_at)
        .then(|| device.reading(now))
        .flatten()
}
const INPUT_IDLE_MS: u32 = 5 * 60 * 1000;

fn input_is_idle(now: u32, last_input: u32) -> bool {
    // LASTINPUTINFO uses 32-bit ticks. Permit wraparound and ignore near-future samples.
    (INPUT_IDLE_MS..u32::MAX - 1000).contains(&now.wrapping_sub(last_input))
}

pub fn session_idle() -> bool {
    use windows_sys::Win32::{
        System::SystemInformation::GetTickCount,
        UI::{Input::KeyboardAndMouse::*, Shell::*},
    };
    unsafe {
        let mut state = QUNS_ACCEPTS_NOTIFICATIONS;
        if SHQueryUserNotificationState(&mut state) >= 0 && state == QUNS_NOT_PRESENT {
            return true;
        }
        let mut input = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        GetLastInputInfo(&mut input) != 0 && input_is_idle(GetTickCount(), input.dwTime)
    }
}
pub fn fullscreen_application() -> bool {
    use windows_sys::Win32::UI::{Shell::*, WindowsAndMessaging::*};
    unsafe {
        let mut state = QUNS_ACCEPTS_NOTIFICATIONS;
        if SHQueryUserNotificationState(&mut state) >= 0
            && matches!(state, QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_APP)
        {
            return true;
        }
        let window = GetForegroundWindow();
        if window.is_null() || window == GetShellWindow() {
            return false;
        }
        let mut rect = std::mem::zeroed();
        let mut monitor: windows_sys::Win32::Graphics::Gdi::MONITORINFO = std::mem::zeroed();
        monitor.cbSize = std::mem::size_of_val(&monitor) as u32;
        let screen = windows_sys::Win32::Graphics::Gdi::MonitorFromWindow(
            window,
            windows_sys::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST,
        );
        if GetWindowRect(window, &mut rect) == 0
            || windows_sys::Win32::Graphics::Gdi::GetMonitorInfoW(screen, &mut monitor) == 0
        {
            return false;
        }
        rect.left <= monitor.rcMonitor.left
            && rect.top <= monitor.rcMonitor.top
            && rect.right >= monitor.rcMonitor.right
            && rect.bottom >= monitor.rcMonitor.bottom
    }
}
fn accepts_notifications() -> bool {
    use windows_sys::Win32::UI::Shell::*;
    unsafe {
        let mut state = QUNS_ACCEPTS_NOTIFICATIONS;
        SHQueryUserNotificationState(&mut state) >= 0
            && state == QUNS_ACCEPTS_NOTIFICATIONS
            && !fullscreen_application()
    }
}
#[cfg(test)]
mod queue_tests {
    use super::*;
    #[test]
    fn successful_alias_rename_keeps_queued_delivery_and_deduplicates_each_queue() {
        let mut queue = OverlayQueue::default();
        for (key, title) in [
            ("alias", "original"),
            ("other", "other"),
            ("canonical", "duplicate"),
        ] {
            queue.enqueue(Notice::new(key, title, "", TEST_ICON, false, false));
            queue.defer(key);
        }
        queue.rename_device("alias", "canonical");
        assert_eq!(queue.pending.len(), 2);
        assert_eq!(queue.pending[0].key, "canonical");
        assert_eq!(queue.pending[0].title, "original");
        assert_eq!(queue.pending[1].key, "other");
        assert_eq!(
            queue.deferred,
            ["canonical".to_owned(), "other".to_owned()]
                .into_iter()
                .collect()
        );
        queue.rename_device("canonical", "canonical");
        assert_eq!(queue.pending.len(), 2);
    }
    #[test]
    fn deferred_alert_rechecks_connection_freshness_power_and_threshold() {
        use crate::battery::{Kind, Power, Reading, Source, Tracked};
        let original = Tracked {
            id: "mouse".into(),
            name: "Mouse".into(),
            source: Source::Mchose,
            kind: Kind::Mouse,
            connection: Some(true),
            observed_at: 100,
            components: Vec::new(),
            clock: std::time::Instant::now(),
            value_clock: std::time::Instant::now(),
            received_at: Some(100),
            value_changed_at: Some(100),
            last: Some((
                Reading {
                    value: crate::battery::Value::reported(15),
                    kind: Kind::Mouse,
                    power: Power::Battery,
                    connected: true,
                },
                100,
            )),
            error: None,
        };
        assert!(eligible_deferred(&original, 20, 100));
        let requested_at = original.value_clock + std::time::Duration::from_millis(1);
        let mut replayed = original.clone();
        replayed.clock = requested_at;
        assert!(verified_deferred_reading(&replayed, requested_at, 100).is_none());
        replayed.value_clock = requested_at;
        assert_eq!(
            verified_deferred_reading(&replayed, requested_at, 100).and_then(Reading::percent),
            Some(15)
        );
        replayed.source = Source::Windows;
        assert!(verified_deferred_reading(&replayed, requested_at, 100).is_none());
        assert!(!eligible_deferred(&original, 0, 100));
        assert!(!eligible_deferred(&original, 10, 100));
        assert!(!eligible_deferred(&original, 20, 401));
        for power in [Power::Charging, Power::Full, Power::Unknown] {
            let mut changed = original.clone();
            changed.last.as_mut().unwrap().0.power = power;
            assert!(!eligible_deferred(&changed, 20, 100));
        }
        let mut changed = original.clone();
        changed.connection = Some(false);
        assert!(!eligible_deferred(&changed, 20, 100));
        changed = original.clone();
        changed.source = Source::Windows;
        assert!(!eligible_deferred(&changed, 20, 100));
        changed = original;
        changed.error = Some("timeout".into());
        assert!(!eligible_deferred(&changed, 20, 100));
    }
    #[test]
    fn queue_replaces_same_device_and_bounds_bursts() {
        let mut queue = OverlayQueue::default();
        for i in 0..20 {
            queue.enqueue(Notice::new(i.to_string(), "", "", TEST_ICON, false, false));
        }
        assert_eq!(queue.pending.len(), crate::battery::MAX_DEVICES);
        queue.enqueue(Notice::new("19", "updated", "", TEST_ICON, false, false));
        assert_eq!(queue.pending.len(), crate::battery::MAX_DEVICES);
        assert_eq!(queue.pending.back().unwrap().title, "updated");
        queue.defer("a");
        queue.defer("a");
        assert_eq!(queue.deferred_count(), 1);
    }
    #[test]
    fn idle_detection_handles_boundary_and_tick_wrap() {
        let last = u32::MAX - 100;
        assert!(!input_is_idle(last.wrapping_add(INPUT_IDLE_MS - 1), last));
        assert!(input_is_idle(last.wrapping_add(INPUT_IDLE_MS), last));
        assert!(!input_is_idle(100, 105));
    }
    #[test]
    fn idle_deferral_keeps_manual_tests_and_deduplicates_fullscreen_replay() {
        let mut queue = OverlayQueue::default();
        for (key, test) in [("mouse", false), ("test", true)] {
            queue.enqueue(Notice::new(key, "", "", TEST_ICON, false, test));
        }
        queue.defer("mouse");
        queue.defer_pending_while_idle();
        assert_eq!(queue.pending.len(), 1);
        assert!(queue.pending[0].test);
        assert!(queue.deferred.is_empty());
        queue.rename_device("mouse", "canonical");
        assert_eq!(
            queue.take_idle_deferred(),
            BTreeSet::from(["canonical".into()])
        );
        assert!(!queue.waiting_for_activity());
    }
}
