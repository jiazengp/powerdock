//! Compiled MCHOSE protocol fingerprints.
use crate::adapters::device::Device;

pub fn mchose_a7(d: &Device) -> bool {
    d.vendor_id == 0x3837
        && [0x1014, 0x1018, 0x4031].contains(&d.product_id)
        && d.usage_page == 0xff01
        && d.usage == 1
        && d.input_bytes == 64
        && d.output_bytes == 64
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseProtocol {
    A7 { wired: bool },
    Yjx { wired: bool },
}
impl MouseProtocol {
    pub fn wired(self) -> bool {
        match self {
            Self::A7 { wired } | Self::Yjx { wired } => wired,
        }
    }
}
pub fn mouse_protocol(d: &Device) -> Option<MouseProtocol> {
    if mchose_a7(d) {
        return Some(MouseProtocol::A7 {
            wired: d.product_id == 0x4031,
        });
    }
    if d.vendor_id != 0x3837
        || d.usage_page != 0xff01
        || d.usage != 0x10
        || d.input_bytes != 65
        || d.output_bytes != 65
    {
        return None;
    }
    // Pinned upstream transport/model table, not display-name matching.
    [
        (0x418c, true),
        (0x4245, false),
        (0x4242, true),
        (0x4012, true),
        (0x4013, true),
    ]
    .into_iter()
    .find_map(|(pid, wired)| (pid == d.product_id).then_some(MouseProtocol::Yjx { wired }))
}
pub fn mouse_wired(device: &Device) -> bool {
    mouse_protocol(device).is_some_and(MouseProtocol::wired)
}
pub fn mchose_keyboard(d: &Device) -> bool {
    d.vendor_id == 0x41e4
        && [0x2001, 0x2004].contains(&d.product_id)
        && d.usage_page == 0xff02
        && d.usage == 2
        && d.input_bytes == 20
        && d.output_bytes == 20
}
pub fn sinowealth_keyboard(d: &Device) -> bool {
    mchose_keyboard(d) && d.product_id == 0x2004
}

#[cfg(test)]
mod tests {
    use super::*;
    fn yjx(pid: u16) -> Device {
        Device {
            path: vec![],
            container: None,
            vendor_id: 0x3837,
            product_id: pid,
            product: "Renamed".into(),
            usage_page: 0xff01,
            usage: 0x10,
            input_bytes: 65,
            output_bytes: 65,
        }
    }
    #[test]
    fn exact_mouse_profiles_do_not_cross_protocol_or_native_report_layout() {
        for (pid, wired) in [
            (0x418c, true),
            (0x4245, false),
            (0x4242, true),
            (0x4012, true),
            (0x4013, true),
        ] {
            let d = yjx(pid);
            assert_eq!(mouse_protocol(&d), Some(MouseProtocol::Yjx { wired }));
        }
        for pid in [0, 0x4031, 0x418d] {
            assert!(!matches!(
                mouse_protocol(&yjx(pid)),
                Some(MouseProtocol::Yjx { .. })
            ));
        }
        let mut d = yjx(0x418c);
        d.input_bytes = 64;
        assert!(mouse_protocol(&d).is_none());
        d.input_bytes = 65;
        d.output_bytes = 64;
        assert!(mouse_protocol(&d).is_none());
        d.output_bytes = 65;
        d.usage = 1;
        assert!(mouse_protocol(&d).is_none());
        d.usage = 0x10;
        d.vendor_id = 0x41e4;
        assert!(mouse_protocol(&d).is_none());
    }
    #[test]
    fn mouse_profile_disable_is_per_device_and_rechecks_revocation() {
        use crate::{
            battery::Source,
            settings::{Profile, ProviderPreferences, Reader},
        };
        let d = yjx(0x4245);
        let original = ProviderPreferences::default();
        let mut on = original.clone();
        on.enabled_experimental
            .insert(Profile::MchoseYjxExperimental);
        assert!(on.mouse_device_enabled(&d));
        assert!(!original.mouse_device_enabled(&d));
        assert!(on.changed_for(&original, &[Source::Mchose]));
        assert!(original.source(Source::Mchose)); // A7 remains enabled: source-wide gate alone is insufficient.
        on.disabled_profiles.insert(Profile::MchoseA7);
        assert!(on.mouse_device_enabled(&d)); // A7 is not a parent switch for another protocol.
        let mut a7 = yjx(0x1018);
        a7.usage = 1;
        a7.input_bytes = 64;
        a7.output_bytes = 64;
        assert!(!on.mouse_device_enabled(&a7));
        assert!(original.mouse_device_enabled(&a7));
        on.disabled_readers.insert(Reader::MchoseHid);
        assert!(!on.mouse_device_enabled(&d));
        assert!(Profile::experimental_variants().contains(&Profile::MchoseYjxExperimental));
        assert!(!Profile::experimental_variants().contains(&Profile::MchoseA7));
    }
    #[test]
    fn keyboard_fingerprint_is_model_and_collection_scoped() {
        let mut d = Device {
            path: vec![],
            container: None,
            vendor_id: 0x41e4,
            product_id: 0x2004,
            product: "Renamed keyboard".into(),
            usage_page: 0xff02,
            usage: 2,
            input_bytes: 20,
            output_bytes: 20,
        };
        assert!(sinowealth_keyboard(&d));
        d.product_id = 0x2203; // Wired uses a different Feature transport.
        assert!(!mchose_keyboard(&d));
        d.product_id = 0x2004;
        d.usage_page = 1;
        assert!(!mchose_keyboard(&d));
    }
    #[test]
    fn opt_in_and_disable_revoke_only_the_new_keyboard_profile() {
        use crate::{
            battery::Source,
            settings::{Profile, ProviderPreferences, Reader},
        };
        let mut d = Device {
            path: vec![],
            container: None,
            vendor_id: 0x41e4,
            product_id: 0x2004,
            product: "Any display name".into(),
            usage_page: 0xff02,
            usage: 2,
            input_bytes: 20,
            output_bytes: 20,
        };
        let original = ProviderPreferences::default();
        assert!(!original.keyboard_device_enabled(&d));
        let mut opted = original.clone();
        opted
            .enabled_experimental
            .insert(Profile::MchoseSinowealthExperimental);
        assert!(opted.keyboard_device_enabled(&d));
        assert!(original.changed_for(&opted, &[Source::MchoseKeyboard]));
        assert!(!original.changed_for(&opted, &[Source::Windows]));
        let mut disabled = opted.clone();
        disabled.disabled_readers.insert(Reader::MchoseKeyboardHid);
        assert!(!disabled.keyboard_device_enabled(&d));
        disabled = opted.clone();
        disabled
            .disabled_profiles
            .insert(Profile::MchoseSinowealthExperimental);
        assert!(!disabled.keyboard_device_enabled(&d));
        assert!(opted.changed_for(&disabled, &[Source::MchoseKeyboard]));
        d.product_id = 0x2001;
        assert!(original.keyboard_device_enabled(&d));
        assert!(disabled.keyboard_device_enabled(&d));
    }
}
