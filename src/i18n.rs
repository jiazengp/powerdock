//! Shared catalog: rust-i18n in the host, generated standard MRT resources in WinUI.
#[cfg(test)]
#[path = "../build_support/locales.rs"]
mod catalog;

use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
pub enum Language {
    #[default]
    #[serde(rename = "system")]
    #[value(name = "system")]
    System,
    #[serde(rename = "zh-CN")]
    #[value(name = "zh-CN")]
    Chinese,
    #[serde(rename = "en-US")]
    #[value(name = "en-US")]
    English,
}
impl Language {
    pub fn locale(self) -> &'static str {
        match self {
            Self::Chinese => "zh-CN",
            Self::English => "en-US",
            Self::System => {
                if unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() } & 0x03ff
                    == 4
                {
                    "zh-CN"
                } else {
                    "en-US"
                }
            }
        }
    }
}
pub fn set_language(language: Language) {
    rust_i18n::set_locale(language.locale());
}
pub fn text(key: &str) -> String {
    rust_i18n::t!(key).to_string()
}
include!(concat!(env!("OUT_DIR"), "/status_keys.rs"));
/// Internal protocol errors retain their stable identifiers; translate at presentation time.
pub fn status(value: &str) -> String {
    if let Some(key) = status_key(value) {
        return text(key);
    }
    if value.contains(" · ") {
        return value
            .split(" · ")
            .map(status)
            .collect::<Vec<_>>()
            .join(" · ");
    }
    if let Some((prefix, detail)) = value.split_once('：')
        && let Some(key) = status_key(prefix)
    {
        return format!("{}: {detail}", text(key));
    }
    value.to_owned()
}
#[cfg(test)]
mod tests {
    #[test]
    fn steelseries_source_is_reachable_through_status_translation() {
        assert_eq!(
            super::status_key(crate::battery::Source::SteelSeries.label()),
            Some("StatusSteelseriesSource")
        );
        assert_eq!(
            rust_i18n::t!("StatusSteelseriesSource", locale = "en-US"),
            "SteelSeries battery · experimental"
        );
    }

    use super::*;
    #[test]
    fn shared_catalog_keys_and_placeholders_match() {
        let zh: serde_json::Value =
            serde_json::from_str(include_str!("../locales/zh-CN.json")).unwrap();
        let en: serde_json::Value =
            serde_json::from_str(include_str!("../locales/en-US.json")).unwrap();
        catalog::validate_catalogs(&[("en-US", en), ("zh-CN", zh)]).unwrap();
        assert_eq!(rust_i18n::t!("Overview", locale = "en-US"), "Devices");
        assert_eq!(rust_i18n::t!("Overview", locale = "zh-CN"), "设备");
        assert!(status_key("电量查询超时").is_some());
        assert_eq!(Language::English.locale(), "en-US");
    }

    #[test]
    fn formatting_keeps_device_names_literal() {
        let name = "鼠标 🎮 %{percent}";
        assert_eq!(
            rust_i18n::t!(
                "NotificationBody",
                locale = "en-US",
                name = name,
                percent = 15
            ),
            "鼠标 🎮 %{percent} is at 15%. Charge it soon."
        );
        assert_eq!(
            rust_i18n::t!(
                "NotificationBody",
                locale = "en-US",
                percent = 15,
                name = name
            ),
            rust_i18n::t!(
                "NotificationBody",
                locale = "en-US",
                name = name,
                percent = 15
            )
        );
        assert!(
            rust_i18n::t!(
                "NotificationBody",
                locale = "zh-CN",
                name = name,
                percent = 15
            )
            .contains(name)
        );
    }
}
