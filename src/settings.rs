mod providers;
pub use providers::{Profile, ProviderPreferences, Reader};
#[cfg(test)]
mod tests;

#[cfg(test)]
use crate::battery::Power;
use crate::battery::Reading;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn wide(value: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    #[default]
    Battery,
    Type,
    Charging,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum NotificationStyle {
    #[default]
    Windows,
    Overlay,
    Both,
}
impl NotificationStyle {
    pub fn windows(self) -> bool {
        self != Self::Overlay
    }
    pub fn overlay(self) -> bool {
        self != Self::Windows
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationPreferences {
    pub style: NotificationStyle,
    pub sound: bool,
    pub after_fullscreen: bool,
    pub position: OverlayPosition,
    pub duration_secs: u64,
    pub monitor: OverlayMonitor,
    pub transparency_percent: u8,
}
pub const OVERLAY_DURATIONS: [u64; 5] = [3, 6, 10, 15, 30];
pub const ALERT_THRESHOLDS: [u8; 4] = [0, 10, 20, 30];
pub const POLL_INTERVALS: [u64; 3] = [30, 60, 120];
pub fn preset_choices<T: std::fmt::Display>(values: &[T]) -> String {
    values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum OverlayPosition {
    #[default]
    TopRight,
    BottomRight,
    TopLeft,
    BottomLeft,
}
impl OverlayPosition {
    pub fn argument(self) -> &'static str {
        match self {
            Self::TopRight => "top-right",
            Self::BottomRight => "bottom-right",
            Self::TopLeft => "top-left",
            Self::BottomLeft => "bottom-left",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum OverlayMonitor {
    #[default]
    Active,
    Primary,
}
impl Default for NotificationPreferences {
    fn default() -> Self {
        Self {
            style: NotificationStyle::Windows,
            sound: true,
            after_fullscreen: true,
            position: OverlayPosition::TopRight,
            duration_secs: 6,
            monitor: OverlayMonitor::Active,
            transparency_percent: 20,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdatePreferences {
    pub auto_check: bool,
    pub auto_download: bool,
    pub preview: bool,
    pub follow_system_proxy: bool,
    pub last_checked_at: u64,
}
impl Default for UpdatePreferences {
    fn default() -> Self {
        Self {
            auto_check: true,
            auto_download: true,
            preview: false,
            follow_system_proxy: true,
            last_checked_at: 0,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub threshold: u8,
    pub interval_secs: u64,
    pub alerts: AlertMemory,
    pub device_alerts: BTreeMap<String, AlertMemory>,
    pub device_thresholds: BTreeMap<String, u8>,
    pub sort_by: SortBy,
    pub sort_order: SortOrder,
    pub language: crate::i18n::Language,
    pub theme: Theme,
    pub reduce_motion: bool,
    pub updates: UpdatePreferences,
    pub notifications: NotificationPreferences,
    pub providers: ProviderPreferences,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            threshold: 20,
            interval_secs: 60,
            alerts: AlertMemory::default(),
            device_alerts: BTreeMap::new(),
            device_thresholds: BTreeMap::new(),
            sort_by: SortBy::Battery,
            sort_order: SortOrder::Asc,
            language: crate::i18n::Language::System,
            theme: Theme::System,
            reduce_motion: false,
            updates: UpdatePreferences::default(),
            notifications: NotificationPreferences::default(),
            providers: ProviderPreferences::default(),
        }
    }
}
impl Settings {
    fn valid(&self) -> bool {
        self.version == 1
            && self.providers.valid()
            && (2..=30).contains(&self.notifications.duration_secs)
            && self.notifications.transparency_percent <= 100
            && self.device_alerts.len() <= 32
            && self.device_thresholds.len() <= 32
            && self
                .device_thresholds
                .iter()
                .all(|(id, threshold)| device_id_valid(id) && ALERT_THRESHOLDS.contains(threshold))
            && self
                .device_alerts
                .keys()
                .all(|id| id.len() <= 64 && id.is_ascii())
            && ALERT_THRESHOLDS.contains(&self.threshold)
            && POLL_INTERVALS.contains(&self.interval_secs)
    }
    pub fn configure_threshold(&mut self, threshold: u8) {
        if self.threshold != threshold {
            self.threshold = threshold;
            self.alerts.latched = false;
            for (id, alert) in &mut self.device_alerts {
                if !self.device_thresholds.contains_key(id) {
                    alert.latched = false;
                }
            }
        }
    }
    pub fn threshold_for(&self, id: &str) -> u8 {
        self.device_thresholds
            .get(id)
            .copied()
            .unwrap_or(self.threshold)
    }
    pub fn configure_device_threshold(
        &mut self,
        id: &str,
        threshold: Option<u8>,
    ) -> Result<(), &'static str> {
        if !device_id_valid(id) || threshold.is_some_and(|v| !ALERT_THRESHOLDS.contains(&v)) {
            return Err("设备 ID 或提醒阈值无效");
        }
        if threshold.is_some()
            && !self.device_thresholds.contains_key(id)
            && self.device_thresholds.len() >= 32
        {
            return Err("单设备提醒设置已达到 32 项上限");
        }
        let before = self.threshold_for(id);
        if let Some(value) = threshold {
            self.device_thresholds.insert(id.to_owned(), value);
        } else {
            self.device_thresholds.remove(id);
        }
        if before != self.threshold_for(id) {
            self.alert_memory(id).latched = false;
        }
        Ok(())
    }
    pub fn alert_memory(&mut self, id: &str) -> &mut AlertMemory {
        let legacy = if self.device_alerts.is_empty() {
            self.alerts.clone()
        } else {
            AlertMemory::default()
        };
        if !self.device_alerts.contains_key(id)
            && self.device_alerts.len() >= 32
            && let Some(oldest) = self
                .device_alerts
                .iter()
                .min_by_key(|(_, a)| a.last_seen)
                .map(|(id, _)| id.clone())
        {
            self.device_alerts.remove(&oldest);
        }
        self.device_alerts.entry(id.to_owned()).or_insert(legacy)
    }
    pub fn merge_device_aliases(&mut self, canonical: &str, aliases: &[String]) {
        for alias in aliases.iter().filter(|id| id.as_str() != canonical) {
            if let Some(old) = self.device_thresholds.remove(alias) {
                let value = self
                    .device_thresholds
                    .entry(canonical.into())
                    .or_insert(old);
                *value = if old == 0 || *value == 0 {
                    0
                } else {
                    (*value).max(old)
                };
            }
            if let Some(old) = self.device_alerts.remove(alias) {
                let current = self.device_alerts.entry(canonical.into()).or_default();
                current.latched |= old.latched;
                current.last_sent = current.last_sent.max(old.last_sent);
                current.last_seen = current.last_seen.max(old.last_seen);
            }
        }
    }
}

pub fn device_id_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AlertMemory {
    pub latched: bool,
    pub last_sent: u64,
    pub last_seen: u64,
}
impl AlertMemory {
    pub fn resume(&mut self, time: u64) {
        if time.saturating_sub(self.last_seen) >= 12 * 3600
            && time.saturating_sub(self.last_sent) >= 12 * 3600
        {
            self.latched = false;
        }
    }
    pub fn eligible(&mut self, reading: &Reading, threshold: u8, time: u64) -> bool {
        self.last_seen = time;
        if threshold == 0 || !reading.linked() {
            return false;
        }
        let Some(percent) = reading.percent() else {
            return false;
        };
        if percent >= threshold + 5 {
            self.latched = false;
        }
        reading.discharging() && percent <= threshold && !self.latched
    }
    pub fn sent(&mut self, time: u64) {
        self.latched = true;
        self.last_sent = time;
    }
}

pub fn root() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("PowerDock")
}
pub fn load() -> (Settings, Option<String>) {
    let legacy = root().with_file_name("MchoseBattery");
    load_from(&root(), &legacy)
}
fn load_from(directory: &Path, legacy: &Path) -> (Settings, Option<String>) {
    let current = directory.join("settings.json");
    let old = legacy.join("settings.json");
    let migrating = !current.exists() && old.exists();
    let path = if migrating { old } else { current };
    let result = (|| {
        let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
        if metadata.len() > 16 * 1024 {
            return Err("设置文件超过大小限制".to_owned());
        }
        let value: Settings = serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if !value.valid() {
            return Err("设置版本、阈值或刷新周期无效".into());
        }
        Ok(value)
    })();
    match result {
        Ok(mut value) => {
            value.notifications.duration_secs = OVERLAY_DURATIONS
                .iter()
                .copied()
                .min_by_key(|preset| {
                    (
                        preset.abs_diff(value.notifications.duration_secs),
                        std::cmp::Reverse(*preset),
                    )
                })
                .unwrap_or(6);
            value.alerts.resume(now());
            for alert in value.device_alerts.values_mut() {
                alert.resume(now());
            }
            let warning = if migrating {
                save_to(&value, directory)
                    .err()
                    .map(|error| format!("旧设置已读取但迁移保存失败：{error}"))
            } else {
                None
            };
            (value, warning)
        }
        Err(_) if !path.exists() => (Settings::default(), None),
        Err(error) => {
            let _ = fs::create_dir_all(directory);
            let _ = fs::copy(
                &path,
                directory.join(format!("settings.invalid-{}.json", now())),
            );
            (
                Settings::default(),
                Some(format!("设置无效，使用默认值：{error}")),
            )
        }
    }
}
pub fn save(settings: &Settings) -> Result<(), String> {
    save_to(settings, &root())
}
fn save_to(settings: &Settings, directory: &Path) -> Result<(), String> {
    if !settings.valid() {
        return Err("拒绝保存无效设置".into());
    }
    let data = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    atomic_write(directory, "settings", &data, 16 * 1024)
}
pub(crate) fn atomic_write(
    directory: &Path,
    name: &str,
    data: &[u8],
    limit: usize,
) -> Result<(), String> {
    if data.len() > limit {
        return Err("配置文件超过大小限制".into());
    }
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let path = directory.join(format!("{name}.json"));
    let temp = directory.join(format!("{name}.tmp"));
    use std::io::Write;
    let mut file = fs::File::create(&temp).map_err(|e| e.to_string())?;
    file.write_all(data)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    let success = unsafe {
        MoveFileExW(
            wide(&temp).as_ptr(),
            wide(&path).as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if success == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}
pub fn log_error(message: &str) {
    crate::diagnostics::error("application_error", serde_json::json!({"error":message}));
}

pub fn identity(path: &Path) -> [u8; 16] {
    let normalized = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_owned())
        .to_string_lossy()
        .to_lowercase();
    let hash = |seed: u64| {
        normalized.as_bytes().iter().fold(seed, |value, byte| {
            (value ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
    };
    let mut bytes = [0; 16];
    bytes[..8].copy_from_slice(&hash(0xcbf29ce484222325).to_le_bytes());
    bytes[8..].copy_from_slice(&hash(0x84222325cbf29ce4).to_le_bytes());
    bytes[7] = (bytes[7] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    bytes
}

#[cfg(test)]
mod migration_tests;
