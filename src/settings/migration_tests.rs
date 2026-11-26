use super::*;

#[test]
fn update_proxy_defaults_to_system_and_preserves_opt_out() {
    let mut old: Settings =
        serde_json::from_str(r#"{"updates":{"preview":true,"auto_download":false}}"#).unwrap();
    assert!(old.updates.follow_system_proxy);
    old.updates.follow_system_proxy = false;
    let loaded: Settings = serde_json::from_str(&serde_json::to_string(&old).unwrap()).unwrap();
    assert!(!loaded.updates.follow_system_proxy);
    assert!(loaded.updates.preview);
    assert!(!loaded.updates.auto_download);
}

#[test]
fn notification_preferences_migrate_and_roundtrip_without_changing_alert_thresholds() {
    let old: Settings = serde_json::from_str(r#"{"version":1,"threshold":30}"#).unwrap();
    assert_eq!(old.notifications.style, NotificationStyle::Windows);
    assert_eq!(old.notifications.transparency_percent, 20);
    assert!(old.notifications.sound && old.notifications.after_fullscreen);
    let json = r#"{"version":1,"threshold":10,"notifications":{"style":"overlay","sound":false,"after_fullscreen":false}}"#;
    let settings: Settings = serde_json::from_str(json).unwrap();
    let loaded: Settings =
        serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
    assert_eq!(loaded.threshold, 10);
    assert_eq!(loaded.notifications.style, NotificationStyle::Overlay);
    assert!(!loaded.notifications.sound);
    assert!(!loaded.notifications.after_fullscreen);
    assert_eq!(loaded.notifications.duration_secs, 6);
    assert_eq!(loaded.notifications.position, OverlayPosition::TopRight);
    for duration in [0, 1, 31, u64::MAX] {
        let mut bad = loaded.clone();
        bad.notifications.duration_secs = duration;
        assert!(!bad.valid());
    }
    assert!(serde_json::from_str::<Settings>(r#"{"notifications":{"style":"invalid"}}"#).is_err());
}
#[test]
fn popup_transparency_roundtrips_and_rejects_invalid_saved_values() {
    for value in [0, 20, 50, 100] {
        let mut settings = Settings::default();
        settings.notifications.transparency_percent = value;
        assert!(settings.valid());
        let loaded: Settings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(loaded.notifications.transparency_percent, value);
        assert_eq!(loaded.threshold, settings.threshold);
    }
    for value in [101, 255] {
        let mut settings = Settings::default();
        settings.notifications.transparency_percent = value;
        assert!(!settings.valid());
    }
    assert!(
        serde_json::from_str::<Settings>(r#"{"notifications":{"transparency_percent":-1}}"#)
            .is_err()
    );
}
#[test]
fn legacy_duration_moves_to_nearest_preset_and_preserves_other_settings() {
    let scratch = std::env::temp_dir().join(format!(
        "PowerDock-duration-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut old = Settings {
        threshold: 30,
        ..Settings::default()
    };
    old.notifications.duration_secs = 8;
    save_to(&old, &scratch).unwrap();
    let (loaded, warning) = load_from(&scratch, &scratch.join("absent"));
    assert!(warning.is_none());
    assert_eq!(loaded.notifications.duration_secs, 10);
    assert_eq!(loaded.threshold, 30);
    fs::remove_dir_all(scratch).unwrap();
}
#[test]
fn product_rename_preserves_preferences_and_never_overwrites_current_settings() {
    let scratch = std::env::temp_dir().join(format!(
        "PowerDock-migration-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let current = scratch.join("PowerDock");
    let legacy = scratch.join("MchoseBattery");
    let mut old = Settings {
        threshold: 30,
        interval_secs: 120,
        ..Settings::default()
    };
    old.configure_device_threshold("mchose-device", Some(10))
        .unwrap();
    save_to(&old, &legacy).unwrap();
    let legacy_bytes = fs::read(legacy.join("settings.json")).unwrap();
    let (migrated, warning) = load_from(&current, &legacy);
    assert!(warning.is_none());
    assert_eq!(migrated.threshold, 30);
    assert_eq!(migrated.interval_secs, 120);
    assert_eq!(migrated.device_thresholds.get("mchose-device"), Some(&10));
    assert!(current.join("settings.json").is_file());
    assert_eq!(
        fs::read(legacy.join("settings.json")).unwrap(),
        legacy_bytes
    );
    let mut newer = migrated;
    newer.threshold = 10;
    save_to(&newer, &current).unwrap();
    let (loaded, warning) = load_from(&current, &legacy);
    assert!(warning.is_none());
    assert_eq!(loaded.threshold, 10);
    fs::remove_dir_all(scratch).unwrap();
}
