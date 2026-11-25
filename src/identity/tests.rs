mod replacement;

use super::*;
#[test]
fn temporary_ids_do_not_repeat_across_process_restarts() {
    const MARKER: &str = "POWERDOCK_REVIEW_TEMP_ID_CHILD";
    if std::env::var_os(MARKER).is_some() {
        println!("{}", temporary_id("same-receiver", 1));
        return;
    }
    let child = || {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "identity::tests::temporary_ids_do_not_repeat_across_process_restarts",
                "--nocapture",
            ])
            .env(MARKER, "1")
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .find(|l| l.starts_with("session-"))
            .unwrap()
            .to_owned()
    };
    assert_ne!(child(), child());
    assert_eq!(
        temporary_id("same-receiver", 1),
        temporary_id("same-receiver", 1)
    );
    assert_ne!(
        temporary_id("same-receiver", 1),
        temporary_id("same-receiver", 2)
    );
}
fn sample(id: &str, proof: Evidence) -> battery::Sample {
    battery::Sample {
        id: id.into(),
        name: id.into(),
        source: battery::Source::Sennheiser,
        kind: battery::Kind::Headset,
        connection: Some(true),
        at: 100,
        clock: std::time::Instant::now(),
        identity: proof,
        session: 1,
        reader_epoch: 1,
        value_observation: None,
        components: Vec::new(),
        result: Ok(battery::Reading {
            value: battery::Value::reported(10),
            kind: battery::Kind::Headset,
            power: battery::Power::Battery,
            connected: true,
        }),
    }
}
#[test]
fn missing_current_evidence_keeps_persisted_alias_and_its_disabled_reminder() {
    let mut directory = Directory::default();
    let proof = Evidence::Strong("proof".into());
    directory.resolve("a", &proof, 1).unwrap();
    directory.resolve("b", &proof, 1).unwrap();
    let mut config = settings::Settings::default();
    config.configure_device_threshold("b", Some(0)).unwrap();
    let mut samples = vec![sample("b", Evidence::Unverified)];
    let r = reconcile(
        &mut Some(directory),
        &mut config,
        &mut samples,
        |_| panic!("existing alias should need no directory write"),
        |_| Ok(()),
    );
    assert_eq!(samples[0].id, "a");
    assert_eq!(config.threshold_for("a"), 0);
    assert!(r.blocked.is_empty());
}
#[test]
fn batch_recanonicalizes_earlier_endpoints_before_migrating_disabled_settings() {
    let mut d = Directory::default();
    d.resolve("a", &Evidence::Strong("old-proof".into()), 1)
        .unwrap();
    d.resolve("b", &Evidence::Strong("shared-proof".into()), 1)
        .unwrap();
    let mut config = settings::Settings::default();
    config.configure_device_threshold("b", Some(0)).unwrap();
    config.alert_memory("b").sent(50);
    let mut samples = vec![
        sample("b", Evidence::Strong("shared-proof".into())),
        sample("a", Evidence::Strong("shared-proof".into())),
    ];
    let r = reconcile(
        &mut Some(d),
        &mut config,
        &mut samples,
        |_| Ok(()),
        |_| Ok(()),
    );
    assert!(samples.iter().all(|s| s.id == "a"));
    assert_eq!(r.endpoints["a"], vec!["b", "a"]);
    assert_eq!(config.threshold_for("a"), 0);
    assert!(config.alert_memory("a").latched);
    assert!(r.blocked.is_empty());
}
#[test]
fn failed_settings_write_blocks_all_endpoints_of_the_physical_device_and_retries() {
    let root =
        std::env::temp_dir().join(format!("powerdock-migration-fault-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir(root.join("settings.tmp")).unwrap();
    let mut d = Directory::default();
    let proof = Evidence::Strong("proof".into());
    d.resolve("a", &proof, 1).unwrap();
    let mut directory = Some(d);
    let mut config = settings::Settings::default();
    config.configure_device_threshold("z", Some(0)).unwrap();
    config.alert_memory("z").sent(50);
    let original = serde_json::to_vec(&config).unwrap();
    std::fs::write(root.join("settings.json"), &original).unwrap();
    let mut samples = vec![sample("a", proof.clone()), sample("z", proof.clone())];
    let r = reconcile(
        &mut directory,
        &mut config,
        &mut samples,
        |d| d.save_to(&root),
        |s| {
            settings::atomic_write(
                &root,
                "settings",
                &serde_json::to_vec(s).unwrap(),
                16 * 1024,
            )
        },
    );
    assert!(samples.iter().all(|s| s.id == "a" && s.result.is_ok()));
    assert!(r.blocked.contains("a"));
    assert!(r.error.is_some());
    assert_eq!(serde_json::to_vec(&config).unwrap(), original);
    assert_eq!(std::fs::read(root.join("settings.json")).unwrap(), original);
    std::fs::remove_dir(root.join("settings.tmp")).unwrap();
    let mut retry = vec![sample("a", proof.clone()), sample("z", proof)];
    let r = reconcile(
        &mut directory,
        &mut config,
        &mut retry,
        |d| d.save_to(&root),
        |s| {
            settings::atomic_write(
                &root,
                "settings",
                &serde_json::to_vec(s).unwrap(),
                16 * 1024,
            )
        },
    );
    assert!(r.blocked.is_empty());
    assert_eq!(config.threshold_for("a"), 0);
    assert!(config.alert_memory("a").latched);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn directory_load_save_and_capacity_failures_cannot_enable_reminders_or_change_settings() {
    let mut config = settings::Settings::default();
    config.configure_device_threshold("old", Some(0)).unwrap();
    config.alert_memory("old").sent(50);
    let before = serde_json::to_vec(&config).unwrap();
    let mut missing = None;
    let mut a = vec![sample("old", Evidence::Route("route".into()))];
    let failed = reconcile(
        &mut missing,
        &mut config,
        &mut a,
        |_| panic!("missing directory cannot save"),
        |_| panic!("must preserve settings"),
    );
    assert!(failed.blocked.contains(&a[0].id));
    assert!(a[0].id.starts_with("session-"));
    let mut present = Some(Directory::default());
    let mut b = vec![sample("old", Evidence::Route("route".into()))];
    let failed = reconcile(
        &mut present,
        &mut config,
        &mut b,
        |_| Err("access denied".into()),
        |_| panic!("must preserve settings"),
    );
    assert!(failed.blocked.contains(&b[0].id));
    assert_eq!(a[0].id, b[0].id);
    assert_eq!(present.as_ref().unwrap().routes.len(), 0);
    let full = Directory {
        routes: (0..256)
            .map(|i| (format!("route-{i}"), Route::default()))
            .collect(),
        ..Directory::default()
    };
    let mut c = vec![sample("old", Evidence::Route("new-route".into()))];
    let failed = reconcile(
        &mut Some(full),
        &mut config,
        &mut c,
        |_| panic!("invalid candidate cannot save"),
        |_| panic!("must preserve settings"),
    );
    assert!(failed.blocked.contains(&c[0].id));
    assert_eq!(serde_json::to_vec(&config).unwrap(), before);
}
#[test]
fn strong_alias_retains_published_id_and_never_merges_by_name() {
    let mut d = Directory::default();
    let proof = Evidence::Strong("container-1".into());
    assert_eq!(d.resolve("bt-old", &proof, 1).unwrap().id, "bt-old");
    let r = d.resolve("usb-new", &proof, 2).unwrap();
    assert_eq!(r.id, "bt-old");
    assert_eq!(d.aliases(&r.id), vec!["usb-new"]);
    assert_eq!(
        d.resolve("bt-other", &Evidence::Strong("container-2".into()), 3)
            .unwrap()
            .id,
        "bt-other"
    );
}
#[test]
fn either_transport_can_upgrade_without_renumbering_and_route_overrides_do_not_transfer() {
    for (first, second) in [("usb-old", "bt-new"), ("bt-old", "usb-new")] {
        let mut directory = Directory::default();
        let proof = Evidence::Strong("physical-proof".into());
        directory.resolve(first, &proof, 1).unwrap();
        let r = directory.resolve(second, &proof, 2).unwrap();
        assert_eq!(r.id, first);
        assert_eq!(directory.canonical(second), first);
    }
    let mut directory = Directory::default();
    let route = Evidence::Route("receiver-slot-1".into());
    let old = directory.resolve("old", &route, 1).unwrap();
    let mut config = settings::Settings::default();
    config.configure_device_threshold(&old.id, Some(0)).unwrap();
    config.alert_memory(&old.id).sent(100);
    let next = directory.resolve("old", &route, 2).unwrap();
    config.merge_device_aliases(&next.id, &directory.aliases(&next.id));
    assert_eq!(config.threshold_for(&next.id), config.threshold);
    assert!(!config.alert_memory(&next.id).latched);
    assert_eq!(config.threshold_for(&old.id), 0);
    assert!(config.alert_memory(&old.id).latched);
}
#[test]
fn route_replacement_and_other_receivers_never_inherit_old_id() {
    let mut d = Directory::default();
    let a = Evidence::Route("receiver-a-slot-1".into());
    let b = Evidence::Route("receiver-b-slot-1".into());
    assert_eq!(d.resolve("old-disabled", &a, 1).unwrap().id, "old-disabled");
    assert_eq!(d.resolve("old-disabled", &a, 1).unwrap().id, "old-disabled");
    assert_eq!(d.resolve("other", &b, 1).unwrap().id, "other");
    let new = d.resolve("old-disabled", &a, 2).unwrap();
    assert_ne!(new.id, "old-disabled");
    assert!(d.aliases(&new.id).is_empty());
    let bytes = serde_json::to_vec(&d).unwrap();
    let mut restarted: Directory = serde_json::from_slice(&bytes).unwrap();
    assert_ne!(restarted.resolve("old-disabled", &a, 2).unwrap().id, new.id);
}
#[test]
fn failed_save_preserves_previous_file() {
    let root = std::env::temp_dir().join(format!("powerdock-identity-{}", std::process::id()));
    let mut d = Directory::default();
    d.resolve("old", &Evidence::Strong("proof".into()), 1)
        .unwrap();
    d.save_to(&root).unwrap();
    let bytes = std::fs::read(root.join("devices.json")).unwrap();
    d.version = 2;
    assert!(d.save_to(&root).is_err());
    assert_eq!(std::fs::read(root.join("devices.json")).unwrap(), bytes);
    d.version = 1;
    // A real filesystem failure must leave the previously committed directory intact.
    std::fs::create_dir(root.join("devices.tmp")).unwrap();
    assert!(d.save_to(&root).is_err());
    assert_eq!(std::fs::read(root.join("devices.json")).unwrap(), bytes);
    std::fs::remove_dir_all(root).unwrap();
}
