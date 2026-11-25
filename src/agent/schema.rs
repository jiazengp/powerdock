//! Stable wire commands and validated preference patches.
use clap::Args;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

fn overlay_duration(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .ok()
        .filter(|value| crate::settings::OVERLAY_DURATIONS.contains(value))
        .ok_or_else(|| {
            format!(
                "Choose an overlay duration from: {} seconds",
                crate::settings::preset_choices(&crate::settings::OVERLAY_DURATIONS)
            )
        })
}
#[derive(Args, Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[command(group(clap::ArgGroup::new("general").required(true).multiple(true).args(["language","theme","reduce_motion","auto_check","auto_download","preview_updates","notification_style","notification_sound","notification_after_fullscreen","overlay_position","overlay_duration","overlay_monitor","overlay_transparency"])))]
pub struct GeneralPatch {
    #[arg(long, value_enum)]
    pub language: Option<crate::i18n::Language>,
    #[arg(long, value_enum)]
    pub theme: Option<crate::settings::Theme>,
    #[arg(long, action=clap::ArgAction::Set)]
    pub reduce_motion: Option<bool>,
    #[arg(long, action=clap::ArgAction::Set)]
    pub auto_check: Option<bool>,
    #[arg(long, action=clap::ArgAction::Set)]
    pub auto_download: Option<bool>,
    #[arg(long, action=clap::ArgAction::Set)]
    pub preview_updates: Option<bool>,
    #[arg(long, value_enum)]
    pub notification_style: Option<crate::settings::NotificationStyle>,
    #[arg(long, action=clap::ArgAction::Set)]
    pub notification_sound: Option<bool>,
    #[arg(long, action=clap::ArgAction::Set)]
    pub notification_after_fullscreen: Option<bool>,
    #[arg(long, value_enum)]
    pub overlay_position: Option<crate::settings::OverlayPosition>,
    #[arg(long,value_parser=overlay_duration)]
    pub overlay_duration: Option<u64>,
    #[arg(long, value_enum)]
    pub overlay_monitor: Option<crate::settings::OverlayMonitor>,
    /// NVIDIA-style popup background transparency: 0 (opaque) to 100 (transparent).
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=100))]
    pub overlay_transparency: Option<u8>,
}
impl GeneralPatch {
    pub fn empty(&self) -> bool {
        self.language.is_none()
            && self.theme.is_none()
            && self.reduce_motion.is_none()
            && self.auto_check.is_none()
            && self.auto_download.is_none()
            && self.preview_updates.is_none()
            && self.notification_style.is_none()
            && self.notification_sound.is_none()
            && self.notification_after_fullscreen.is_none()
            && self.overlay_position.is_none()
            && self.overlay_duration.is_none()
            && self.overlay_monitor.is_none()
            && self.overlay_transparency.is_none()
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.empty() {
            return Err("At least one preference is required");
        }
        if self
            .overlay_duration
            .is_some_and(|duration| !crate::settings::OVERLAY_DURATIONS.contains(&duration))
        {
            return Err("Choose a supported overlay duration preset");
        }
        if self.overlay_transparency.is_some_and(|value| value > 100) {
            return Err("Background transparency must be between 0 and 100 percent");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    ProvidersGet {},
    ProviderReaderSet {
        reader: crate::settings::Reader,
        enabled: bool,
    },
    ProviderProfileSet {
        profile: crate::settings::Profile,
        enabled: bool,
        #[serde(default)]
        experimental: bool,
    },
    Status {},
    Devices {
        #[serde(default)]
        all: bool,
    },
    DevicePage {
        all: bool,
        cursor: Option<String>,
        page_size: u16,
    },
    DeviceCatalog {
        #[serde(default)]
        cursor: Option<String>,
        #[serde(default = "default_page_size")]
        page_size: u16,
    },
    Refresh {
        id: Option<String>,
    },
    DeviceReminderGet {
        id: String,
    },
    DeviceReminderSet {
        id: String,
        threshold: Option<u8>,
    },
    SettingsGet {},
    GeneralSet {
        #[serde(flatten)]
        patch: GeneralPatch,
    },
    NotificationTest {},
    UpdateStatus {},
    UpdateCheck {
        download: bool,
    },
    UpdateApply {},
    SettingsSet {
        threshold: Option<u8>,
        interval_secs: Option<u64>,
        startup: Option<bool>,
        sort_by: Option<crate::settings::SortBy>,
        sort_order: Option<crate::settings::SortOrder>,
    },
    Show {
        settings: bool,
        #[serde(default)]
        menu: bool,
    },
    Quit {},
}
impl Operation {
    pub fn snapshot_only(&self) -> bool {
        matches!(
            self,
            Self::Status {}
                | Self::ProvidersGet {}
                | Self::Devices { .. }
                | Self::DevicePage { .. }
                | Self::DeviceCatalog { .. }
                | Self::SettingsGet {}
                | Self::DeviceReminderGet { .. }
                | Self::UpdateStatus {}
        )
    }
    pub fn preview_allowed(&self) -> bool {
        self.snapshot_only() || matches!(self, Self::Show { .. })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u8,
    pub command: Operation,
}
pub fn success(data: Value) -> Value {
    json!({"schema_version":1,"ok":true,"data":data})
}
pub fn failure(code: &str, message: impl AsRef<str>) -> Value {
    json!({"schema_version":1,"ok":false,"error":{"code":code,"message":message.as_ref()}})
}
fn default_page_size() -> u16 {
    32
}
