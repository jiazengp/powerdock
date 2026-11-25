//! Compiled reader/profile registry and effective provider eligibility.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum Reader {
    LogitechHidpp,
    HidStandard,
    Wgi,
    XInput,
    MchoseHid,
    MchoseUsbHid,
    MchoseKeyboardHid,
    SennheiserGaia,
    BluetoothGatt,
    WindowsMetadata,
    SteelseriesHid,
}
impl Reader {
    pub fn all() -> &'static [Self] {
        <Self as clap::ValueEnum>::value_variants()
    }
    /// Public control IDs are stable; the legacy persisted XInput enum remains x_input.
    pub fn id(self) -> &'static str {
        match self {
            Self::SteelseriesHid => "steelseries_hid",
            Self::LogitechHidpp => "logitech_hidpp",
            Self::HidStandard => "hid_standard",
            Self::Wgi => "wgi",
            Self::XInput => "xinput",
            Self::MchoseHid => "mchose_hid",
            Self::MchoseUsbHid => "mchose_usb_hid",
            Self::MchoseKeyboardHid => "mchose_keyboard_hid",
            Self::SennheiserGaia => "sennheiser_gaia",
            Self::BluetoothGatt => "bluetooth_gatt",
            Self::WindowsMetadata => "windows_metadata",
        }
    }
    pub fn for_source(source: crate::battery::Source) -> Option<Self> {
        use crate::battery::Source;
        Some(match source {
            Source::SteelSeries => Self::SteelseriesHid,
            Source::LogitechHidpp => Self::LogitechHidpp,
            Source::HidStandard => Self::HidStandard,
            Source::Wgi => Self::Wgi,
            Source::XInput => Self::XInput,
            Source::Mchose => Self::MchoseHid,
            Source::MchoseUsb => Self::MchoseUsbHid,
            Source::MchoseKeyboard => Self::MchoseKeyboardHid,
            Source::Sennheiser => Self::SennheiserGaia,
            Source::BluetoothGatt => Self::BluetoothGatt,
            Source::Windows => Self::WindowsMetadata,
            _ => return None,
        })
    }
}
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    LogitechHidppExperimental,
    HidStandardExperimental,
    Bas11Experimental,
    MchoseA7,
    MchoseYjxExperimental,
    MchoseKeyboard,
    MchoseSinowealthExperimental,
    SennheiserMomentum4,
    SennheiserFamilyExperimental,
    SteelseriesDirectExperimental,
}
impl Profile {
    pub fn all() -> &'static [Self] {
        <Self as clap::ValueEnum>::value_variants()
    }
    pub fn experimental_variants() -> Vec<Self> {
        Self::all()
            .iter()
            .copied()
            .filter(|p| p.experimental())
            .collect()
    }
    pub fn experimental(self) -> bool {
        matches!(
            self,
            Self::SteelseriesDirectExperimental
                | Self::MchoseYjxExperimental
                | Self::SennheiserFamilyExperimental
                | Self::MchoseSinowealthExperimental
                | Self::Bas11Experimental
                | Self::HidStandardExperimental
                | Self::LogitechHidppExperimental
        )
    } // Experimental profiles are never enabled by default.
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderPreferences {
    pub disabled_readers: BTreeSet<Reader>,
    pub disabled_profiles: BTreeSet<Profile>,
    pub enabled_experimental: BTreeSet<Profile>,
}
impl ProviderPreferences {
    pub fn valid(&self) -> bool {
        self.disabled_readers.len() <= 64
            && self.disabled_profiles.len() <= 64
            && self.enabled_experimental.len() <= 64
            && self.enabled_experimental.iter().all(|p| p.experimental())
    }
    pub fn reader(&self, reader: Reader) -> bool {
        !self.disabled_readers.contains(&reader)
    }
    pub fn profile(&self, profile: Profile) -> bool {
        !self.disabled_profiles.contains(&profile)
            && (!profile.experimental() || self.enabled_experimental.contains(&profile))
    }
    pub fn source(&self, source: crate::battery::Source) -> bool {
        use crate::battery::Source;
        if Reader::for_source(source).is_some_and(|reader| !self.reader(reader)) {
            return false;
        }
        match source {
            Source::SteelSeries => self.profile(Profile::SteelseriesDirectExperimental),
            Source::LogitechHidpp => self.profile(Profile::LogitechHidppExperimental),
            Source::HidStandard => self.profile(Profile::HidStandardExperimental),
            Source::Mchose | Source::MchoseUsb => {
                self.profile(Profile::MchoseA7) || self.profile(Profile::MchoseYjxExperimental)
            }
            Source::MchoseKeyboard => self.profile(Profile::MchoseKeyboard),
            Source::Sennheiser => self.profile(Profile::SennheiserMomentum4),
            _ => true,
        }
    }
    pub fn mouse_device_enabled(&self, device: &crate::adapters::device::Device) -> bool {
        use crate::adapters::mchose::profiles::{MouseProtocol, mouse_protocol};
        let Some(protocol) = mouse_protocol(device) else {
            return false;
        };
        self.reader(if protocol.wired() {
            Reader::MchoseUsbHid
        } else {
            Reader::MchoseHid
        }) && self.profile(match protocol {
            MouseProtocol::A7 { .. } => Profile::MchoseA7,
            MouseProtocol::Yjx { .. } => Profile::MchoseYjxExperimental,
        })
    }
    pub fn keyboard_device_enabled(&self, device: &crate::adapters::device::Device) -> bool {
        use crate::adapters::mchose::profiles::{mchose_keyboard, sinowealth_keyboard};
        self.source(crate::battery::Source::MchoseKeyboard)
            && mchose_keyboard(device)
            && (!sinowealth_keyboard(device) || self.profile(Profile::MchoseSinowealthExperimental))
    }
    pub fn changed_for(&self, next: &Self, sources: &[crate::battery::Source]) -> bool {
        sources.iter().any(|s| self.source(*s) != next.source(*s))
            || (sources.iter().any(|s| {
                matches!(
                    s,
                    crate::battery::Source::Mchose | crate::battery::Source::MchoseUsb
                )
            }) && (self.profile(Profile::MchoseA7) != next.profile(Profile::MchoseA7)
                || self.profile(Profile::MchoseYjxExperimental)
                    != next.profile(Profile::MchoseYjxExperimental)))
            || (sources.contains(&crate::battery::Source::Sennheiser)
                && self.profile(Profile::SennheiserFamilyExperimental)
                    != next.profile(Profile::SennheiserFamilyExperimental))
            || (sources.contains(&crate::battery::Source::MchoseKeyboard)
                && self.profile(Profile::MchoseSinowealthExperimental)
                    != next.profile(Profile::MchoseSinowealthExperimental))
            || (sources.contains(&crate::battery::Source::BluetoothGatt)
                && self.profile(Profile::Bas11Experimental)
                    != next.profile(Profile::Bas11Experimental))
    }
}
