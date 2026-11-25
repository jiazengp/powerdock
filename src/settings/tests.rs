use super::*;

#[test]
fn reader_registry_keeps_legacy_ids_and_disabled_reader_precedence() {
    use crate::battery::Source;
    let ids: std::collections::BTreeSet<_> =
        Reader::all().iter().map(|reader| reader.id()).collect();
    assert_eq!(ids.len(), Reader::all().len());
    assert_eq!(Reader::XInput.id(), "xinput");
    assert_eq!(serde_json::to_value(Reader::XInput).unwrap(), "x_input");
    let mut enabled = ProviderPreferences::default();
    enabled
        .enabled_experimental
        .extend(Profile::experimental_variants());
    for source in [
        Source::SteelSeries,
        Source::LogitechHidpp,
        Source::HidStandard,
        Source::Wgi,
        Source::XInput,
        Source::Mchose,
        Source::MchoseUsb,
        Source::MchoseKeyboard,
        Source::Sennheiser,
        Source::BluetoothGatt,
        Source::Windows,
    ] {
        let reader = Reader::for_source(source).unwrap();
        assert!(ids.contains(reader.id()));
        assert!(enabled.source(source));
        let mut disabled = enabled.clone();
        disabled.disabled_readers.insert(reader);
        assert!(!disabled.source(source));
    }
    assert_eq!(Reader::for_source(Source::Preview), None);
}
#[test]
fn identity_upgrade_preserves_disabled_latches_and_merges_only_explicit_thresholds() {
    for (left, right, want) in [
        (Some(10), Some(30), 30),
        (Some(0), Some(30), 0),
        (Some(20), Some(0), 0),
        (None, Some(10), 10),
    ] {
        let mut s = Settings::default();
        if let Some(v) = left {
            s.configure_device_threshold("canonical", Some(v)).unwrap();
        }
        if let Some(v) = right {
            s.configure_device_threshold("alias", Some(v)).unwrap();
        }
        s.device_alerts.insert(
            "alias".into(),
            AlertMemory {
                latched: true,
                last_sent: 300,
                last_seen: 400,
            },
        );
        s.device_alerts.insert(
            "canonical".into(),
            AlertMemory {
                latched: false,
                last_sent: 350,
                last_seen: 200,
            },
        );
        s.merge_device_aliases("canonical", &["alias".into()]);
        assert_eq!(s.threshold_for("canonical"), want);
        assert!(!s.device_thresholds.contains_key("alias"));
        assert!(s.device_alerts["canonical"].latched);
        assert_eq!(s.device_alerts["canonical"].last_sent, 350);
        assert_eq!(s.device_alerts["canonical"].last_seen, 400);
    }
    let mut s = Settings::default();
    s.configure_device_threshold("canonical", Some(10)).unwrap();
    s.merge_device_aliases("canonical", &["inherits".into()]);
    assert_eq!(s.threshold_for("canonical"), 10);
}
#[test]
fn experimental_standards_are_closed_by_default_and_revoke_affected_sources() {
    let original = ProviderPreferences::default();
    assert!(!original.source(crate::battery::Source::HidStandard));
    assert!(!original.profile(Profile::Bas11Experimental));
    assert!(!original.source(crate::battery::Source::LogitechHidpp));
    let mut logitech = original.clone();
    logitech
        .enabled_experimental
        .insert(Profile::LogitechHidppExperimental);
    assert!(logitech.source(crate::battery::Source::LogitechHidpp));
    assert!(original.changed_for(&logitech, &[crate::battery::Source::LogitechHidpp]));
    logitech.disabled_readers.insert(Reader::LogitechHidpp);
    assert!(!logitech.source(crate::battery::Source::LogitechHidpp));
    let mut next = original.clone();
    next.enabled_experimental
        .insert(Profile::HidStandardExperimental);
    assert!(next.source(crate::battery::Source::HidStandard));
    assert!(original.changed_for(&next, &[crate::battery::Source::HidStandard]));
    assert!(!original.changed_for(&next, &[crate::battery::Source::Mchose]));
    next.disabled_readers.insert(Reader::HidStandard);
    assert!(!next.source(crate::battery::Source::HidStandard));
    next.enabled_experimental.insert(Profile::Bas11Experimental);
    assert!(original.changed_for(&next, &[crate::battery::Source::BluetoothGatt]));
    assert!(next.changed_for(&original, &[crate::battery::Source::BluetoothGatt]));
    assert_eq!(
        serde_json::from_str::<ProviderPreferences>("{}").unwrap(),
        original
    );
}
#[test]
fn provider_controls_roundtrip_strict_allowlist_and_reader_disable_wins() {
    let mut s: Settings = serde_json::from_str(r#"{"threshold":10,"interval_secs":120}"#).unwrap();
    assert_eq!(s.providers, ProviderPreferences::default());
    s.providers.disabled_readers.insert(Reader::MchoseHid);
    assert!(!s.providers.source(crate::battery::Source::Mchose));
    assert!(s.providers.source(crate::battery::Source::MchoseUsb));
    s.providers
        .disabled_profiles
        .insert(Profile::SennheiserMomentum4);
    assert!(!s.providers.source(crate::battery::Source::Sennheiser));
    let decoded: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(decoded.providers, s.providers);
    assert_eq!(decoded.threshold, 10);
    assert_eq!(decoded.interval_secs, 120);
    assert!(
        serde_json::from_str::<ProviderPreferences>(r#"{"disabled_readers":["arbitrary_opcode"]}"#)
            .is_err()
    );
    s.providers.enabled_experimental.insert(Profile::MchoseA7);
    assert!(!s.valid());
}
#[test]
fn per_device_thresholds_inherit_disable_and_preserve_other_devices() {
    let mut settings = Settings::default();
    settings.alert_memory("mouse").latched = true;
    settings.alert_memory("keyboard").latched = true;
    settings
        .configure_device_threshold("mouse", Some(10))
        .unwrap();
    assert_eq!(settings.threshold_for("mouse"), 10);
    assert_eq!(settings.threshold_for("keyboard"), 20);
    assert!(!settings.device_alerts["mouse"].latched);
    assert!(settings.device_alerts["keyboard"].latched);
    settings.device_alerts.get_mut("mouse").unwrap().latched = true;
    settings.configure_threshold(30);
    assert!(settings.device_alerts["mouse"].latched);
    assert!(!settings.device_alerts["keyboard"].latched);
    settings
        .configure_device_threshold("mouse", Some(0))
        .unwrap();
    assert_eq!(settings.threshold_for("mouse"), 0);
    settings.configure_device_threshold("mouse", None).unwrap();
    assert_eq!(settings.threshold_for("mouse"), 30);
    assert!(!settings.device_thresholds.contains_key("mouse"));
}
#[test]
fn device_threshold_roundtrip_bounds_and_unchanged_value_does_not_rearm() {
    let mut settings: Settings = serde_json::from_str(r#"{"version":1}"#).unwrap();
    assert!(settings.device_thresholds.is_empty());
    settings
        .configure_device_threshold("mouse", Some(20))
        .unwrap();
    settings.alert_memory("mouse").latched = true;
    settings
        .configure_device_threshold("mouse", Some(20))
        .unwrap();
    assert!(settings.device_alerts["mouse"].latched);
    let saved = serde_json::to_string(&settings).unwrap();
    let loaded: Settings = serde_json::from_str(&saved).unwrap();
    assert_eq!(loaded.threshold_for("mouse"), 20);
    assert!(
        settings
            .configure_device_threshold("../invalid", Some(10))
            .is_err()
    );
    assert!(
        settings
            .configure_device_threshold("mouse", Some(101))
            .is_err()
    );
    for i in 0..31 {
        settings
            .configure_device_threshold(&format!("device-{i}"), Some(10))
            .unwrap();
    }
    assert!(
        settings
            .configure_device_threshold("extra", Some(10))
            .is_err()
    );
    settings.configure_device_threshold("mouse", None).unwrap();
    assert!(
        settings
            .configure_device_threshold("extra", Some(10))
            .is_ok()
    );
    assert!(settings.valid());
}
#[test]
fn old_config_defaults_to_battery_ascending_and_sort_choices_roundtrip() {
    let mut settings: Settings =
        serde_json::from_str(r#"{"version":1,"threshold":10,"interval_secs":120}"#).unwrap();
    assert_eq!(settings.sort_by, SortBy::Battery);
    assert_eq!(settings.sort_order, SortOrder::Asc);
    settings.sort_by = SortBy::Charging;
    settings.sort_order = SortOrder::Desc;
    let saved = serde_json::to_string(&settings).unwrap();
    let loaded: Settings = serde_json::from_str(&saved).unwrap();
    assert_eq!(loaded.sort_by, SortBy::Charging);
    assert_eq!(loaded.sort_order, SortOrder::Desc);
    assert_eq!(loaded.threshold, 10);
    assert_eq!(loaded.interval_secs, 120);
    assert!(serde_json::from_str::<Settings>(r#"{"sort_by":"invalid"}"#).is_err());
}
fn low() -> Reading {
    Reading {
        value: crate::battery::Value::reported(18),
        kind: crate::battery::Kind::Mouse,
        power: Power::Battery,
        connected: true,
    }
}
#[test]
fn suppresses_repeat_and_short_restart() {
    let mut a = AlertMemory::default();
    assert!(a.eligible(&low(), 20, 100));
    a.sent(100);
    a.resume(150);
    assert!(!a.eligible(&low(), 20, 150));
    let encoded = serde_json::to_string(&a).unwrap();
    let mut b: AlertMemory = serde_json::from_str(&encoded).unwrap();
    b.resume(200);
    assert!(!b.eligible(&low(), 20, 200));
}
#[test]
fn hysteresis_and_long_offline_rearm() {
    let mut a = AlertMemory {
        latched: true,
        last_sent: 100,
        last_seen: 100,
    };
    let mut r = low();
    r.value = crate::battery::Value::reported(24);
    assert!(!a.eligible(&r, 20, 110));
    r.value = crate::battery::Value::reported(25);
    a.eligible(&r, 20, 120);
    assert!(a.eligible(&low(), 20, 130));
    a.sent(130);
    a.last_seen = 130;
    a.resume(130 + 12 * 3600);
    assert!(a.eligible(&low(), 20, 130 + 12 * 3600));
}
#[test]
fn charge_unknown_and_disabled_do_not_alert() {
    for power in [Power::Charging, Power::Full, Power::Unknown] {
        let mut r = low();
        r.power = power;
        assert!(!AlertMemory::default().eligible(&r, 20, 100));
    }
    assert!(!AlertMemory::default().eligible(&low(), 0, 100));
}
#[test]
fn threshold_change_is_new_evaluation() {
    let mut s = Settings::default();
    s.alerts.latched = true;
    s.configure_threshold(30);
    assert!(!s.alerts.latched);
}
#[test]
fn config_rejects_busy_loop_and_unknown_version() {
    let mut s = Settings {
        interval_secs: 0,
        ..Settings::default()
    };
    assert!(!s.valid());
    s.interval_secs = 60;
    s.version = 99;
    assert!(!s.valid());
}
#[test]
fn per_device_alert_memory_is_independent_and_old_settings_load() {
    let mut s: Settings = serde_json::from_str(
        r#"{"version":1,"threshold":20,"interval_secs":60,"alerts":{"latched":true}}"#,
    )
    .unwrap();
    assert!(s.device_alerts.is_empty());
    let mut first = AlertMemory::default();
    assert!(first.eligible(&low(), 20, 100));
    first.sent(100);
    s.device_alerts.insert("a".into(), first);
    assert!(
        s.device_alerts
            .entry("b".into())
            .or_default()
            .eligible(&low(), 20, 100)
    );
    assert!(
        !s.device_alerts
            .get_mut("a")
            .unwrap()
            .eligible(&low(), 20, 100)
    );
    s.configure_threshold(30);
    assert!(s.device_alerts.values().all(|m| !m.latched));
}
#[test]
fn portable_identity_stable_and_path_specific() {
    assert_eq!(
        identity(Path::new("D:/a/tool.exe")),
        identity(Path::new("d:/A/tool.exe"))
    );
    assert_ne!(
        identity(Path::new("D:/a/tool.exe")),
        identity(Path::new("D:/b/tool.exe"))
    );
}
