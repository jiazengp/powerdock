use super::*;

#[test]
fn exclusive_units_do_not_inherit_overrides_when_a_usb_route_is_reused() {
    let mut directory = Directory::default();
    let mut config = settings::Settings::default();
    let first = directory
        .resolve("usb-route", &Evidence::Strong("os-container".into()), 1)
        .unwrap()
        .id;
    config.configure_device_threshold(&first, Some(0)).unwrap();
    config.alert_memory(&first).latched = true;
    let unit_a = directory
        .resolve("usb-route", &Evidence::Unit("unit-a".into()), 1)
        .unwrap()
        .id;
    assert_eq!(unit_a, first);
    assert_eq!(
        directory
            .resolve(
                "other-os-alias",
                &Evidence::Strong("os-container".into()),
                1
            )
            .unwrap()
            .id,
        unit_a
    );
    let unit_b = directory
        .resolve("usb-route", &Evidence::Unit("unit-b".into()), 2)
        .unwrap()
        .id;
    assert_ne!(unit_a, unit_b);
    let fallback = directory
        .resolve("usb-route", &Evidence::Strong("os-container".into()), 2)
        .unwrap()
        .id;
    assert_ne!(fallback, unit_a);
    assert_ne!(fallback, unit_b);
    assert_eq!(config.threshold_for(&fallback), 20);
    for alias in ["other-os-alias", "new-os-alias"] {
        let alternate = directory
            .resolve(alias, &Evidence::Strong("os-container".into()), 2)
            .unwrap()
            .id;
        assert_ne!(alternate, unit_a);
        assert_ne!(alternate, unit_b);
        assert_eq!(config.threshold_for(&alternate), 20);
    }
    assert_eq!(
        directory
            .resolve("usb-route", &Evidence::Strong("os-container".into()), 2)
            .unwrap()
            .id,
        fallback
    );
    assert_eq!(config.threshold_for(&unit_a), 0);
    assert_eq!(config.threshold_for(&unit_b), 20);
    assert!(config.device_alerts[&unit_a].latched);
    assert!(!config.device_alerts.contains_key(&unit_b));
    let wire_a = directory
        .resolve("other-wireless-route", &Evidence::Unit("unit-a".into()), 3)
        .unwrap()
        .id;
    assert_eq!(wire_a, unit_a);
    let mut reloaded: Directory =
        serde_json::from_str(&serde_json::to_string(&directory).unwrap()).unwrap();
    reloaded.validate().unwrap();
    let after_restart = reloaded
        .resolve("usb-route", &Evidence::Strong("os-container".into()), 4)
        .unwrap()
        .id;
    assert_ne!(after_restart, unit_a);
    assert_ne!(after_restart, unit_b);
    assert_eq!(config.threshold_for(&after_restart), 20);
    assert_ne!(
        reloaded
            .resolve(
                "other-os-alias",
                &Evidence::Strong("os-container".into()),
                4
            )
            .unwrap()
            .id,
        unit_a
    );
    assert_eq!(
        reloaded
            .resolve("usb-route", &Evidence::Unit("unit-b".into()), 4)
            .unwrap()
            .id,
        unit_b
    );
    assert_eq!(
        reloaded
            .resolve("usb-route", &Evidence::Unit("unit-a".into()), 5)
            .unwrap()
            .id,
        unit_a
    );
}
fn reused_unit_directory() -> (Directory, settings::Settings, String, String) {
    let mut directory = Directory::default();
    directory
        .resolve(
            "direct-route",
            &Evidence::Route("shared-route-key".into()),
            1,
        )
        .unwrap();
    directory
        .resolve("direct-route", &Evidence::Strong("os-container".into()), 1)
        .unwrap();
    let a = directory
        .resolve("direct-route", &Evidence::Unit("unit-a".into()), 1)
        .unwrap()
        .id;
    directory
        .resolve("old-alias", &Evidence::Strong("os-container".into()), 1)
        .unwrap();
    let b = directory
        .resolve("direct-route", &Evidence::Unit("unit-b".into()), 2)
        .unwrap()
        .id;
    let mut config = settings::Settings::default();
    config.configure_device_threshold(&a, Some(0)).unwrap();
    config.alert_memory(&a).sent(50);
    config.configure_device_threshold(&b, Some(30)).unwrap();
    (directory, config, a, b)
}
#[test]
fn all_non_unit_fallbacks_stay_separate_after_reconcile_and_restart() {
    let (directory, config, a, b) = reused_unit_directory();
    for proof in [Evidence::Route("old-alias".into()), Evidence::Unverified] {
        let mut old_format = directory.clone();
        old_format.ambiguous_units = BTreeSet::from(["direct-route".into()]);
        let mut old_format =
            Some(serde_json::from_str(&serde_json::to_string(&old_format).unwrap()).unwrap());
        let mut settings = config.clone();
        let mut samples = vec![sample("old-alias", proof)];
        let result = reconcile(
            &mut old_format,
            &mut settings,
            &mut samples,
            |_| Ok(()),
            |_| panic!("must not migrate historical overrides"),
        );
        assert!(result.error.is_none());
        assert_ne!(samples[0].id, a);
        assert_ne!(samples[0].id, b);
        assert_eq!(settings.threshold_for(&samples[0].id), 20);
    }
    for restarted in [false, true] {
        let mut directory = Some(if restarted {
            serde_json::from_str(&serde_json::to_string(&directory).unwrap()).unwrap()
        } else {
            directory.clone()
        });
        let mut config = config.clone();
        let original = serde_json::to_vec(&config).unwrap();
        for observed in ["direct-route", "old-alias", "new-shared-alias"] {
            let mut fallback = None;
            for proof in [
                Evidence::Strong("os-container".into()),
                Evidence::Route(observed.into()),
                Evidence::Unverified,
            ] {
                let mut samples = vec![sample(observed, proof)];
                samples[0].session = 3;
                let result = reconcile(
                    &mut directory,
                    &mut config,
                    &mut samples,
                    |_| Ok(()),
                    |_| panic!("fallback must not migrate historical overrides"),
                );
                assert!(result.error.is_none());
                assert!(result.blocked.is_empty());
                let id = &samples[0].id;
                assert_ne!(id, &a, "{observed}, restart={restarted}");
                assert_ne!(id, &b);
                assert_eq!(config.threshold_for(id), 20);
                assert!(!config.device_alerts.contains_key(id));
                assert_eq!(directory.as_ref().unwrap().canonical(id), *id);
                if let Some(previous) = &fallback {
                    assert_eq!(id, previous);
                }
                fallback = Some(id.clone());
            }
        }
        // A route key can still hold the old unit alias from before the unit
        // observations, even when the new observation uses another endpoint ID.
        let mut samples = vec![sample(
            "different-endpoint",
            Evidence::Route("shared-route-key".into()),
        )];
        let result = reconcile(
            &mut directory,
            &mut config,
            &mut samples,
            |_| Ok(()),
            |_| panic!("must not migrate"),
        );
        assert!(result.error.is_none());
        assert_ne!(samples[0].id, a);
        assert_ne!(samples[0].id, b);
        assert_eq!(serde_json::to_vec(&config).unwrap(), original);
        for (proof, id) in [("unit-a", &a), ("unit-b", &b)] {
            let mut samples = vec![sample("direct-route", Evidence::Unit(proof.into()))];
            reconcile(
                &mut directory,
                &mut config,
                &mut samples,
                |_| Ok(()),
                |_| Ok(()),
            );
            assert_eq!(&samples[0].id, id);
        }
        assert_eq!(config.threshold_for(&a), 0);
        assert!(config.device_alerts[&a].latched);
        assert_eq!(config.threshold_for(&b), 30);
    }
}
#[test]
fn new_unit_evidence_fences_earlier_fallbacks_in_any_batch_order() {
    for evidence in [
        Evidence::Strong("os-container".into()),
        Evidence::Route("a-earlier-alias".into()),
        Evidence::Unverified,
    ] {
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let mut directory = Directory::default();
            directory
                .resolve("direct-route", &Evidence::Strong("os-container".into()), 1)
                .unwrap();
            let a = directory
                .resolve("direct-route", &Evidence::Unit("unit-a".into()), 1)
                .unwrap()
                .id;
            directory
                .resolve(
                    "a-earlier-alias",
                    &Evidence::Strong("os-container".into()),
                    1,
                )
                .unwrap();
            let mut config = settings::Settings::default();
            config.configure_device_threshold(&a, Some(0)).unwrap();
            config.alert_memory(&a).sent(50);
            let original = serde_json::to_vec(&config).unwrap();
            let input = [
                sample("a-earlier-alias", evidence.clone()),
                sample("direct-route", Evidence::Unit("unit-b".into())),
                sample("still-unit-a", Evidence::Unit("unit-a".into())),
            ];
            let mut samples: Vec<_> = order.iter().map(|i| input[*i].clone()).collect();
            let result = reconcile(
                &mut Some(directory),
                &mut config,
                &mut samples,
                |_| Ok(()),
                |_| panic!("historical settings must not move"),
            );
            assert!(result.error.is_none());
            let fallback = samples
                .iter()
                .find(|s| s.name == "a-earlier-alias")
                .unwrap();
            let b = samples.iter().find(|s| s.name == "direct-route").unwrap();
            let real_a = samples.iter().find(|s| s.name == "still-unit-a").unwrap();
            assert_ne!(fallback.id, a, "order={order:?}, evidence={evidence:?}");
            assert_ne!(fallback.id, b.id);
            assert_ne!(b.id, a);
            assert_eq!(real_a.id, a);
            assert_eq!(config.threshold_for(&fallback.id), 20);
            assert_eq!(config.threshold_for(&b.id), 20);
            assert_eq!(config.threshold_for(&a), 0);
            assert!(config.device_alerts[&a].latched);
            assert_eq!(serde_json::to_vec(&config).unwrap(), original);
        }
    }
}
#[test]
fn ambiguous_fallback_save_failure_blocks_without_resetting_unit_settings() {
    let (initial, mut config, a, _) = reused_unit_directory();
    let mut directory = Some(initial);
    for invalid in [
        Evidence::Route("bad/path".into()),
        Evidence::Strong("bad/path".into()),
        Evidence::Unit("bad/path".into()),
    ] {
        let before = serde_json::to_vec(&directory).unwrap();
        assert!(
            directory
                .as_mut()
                .unwrap()
                .resolve("direct-route", &invalid, 3)
                .is_err()
        );
        assert_eq!(serde_json::to_vec(&directory).unwrap(), before);
    }
    let before_directory = serde_json::to_vec(&directory).unwrap();
    let before_config = serde_json::to_vec(&config).unwrap();
    let mut samples = vec![sample(
        "direct-route",
        Evidence::Route("direct-route".into()),
    )];
    let result = reconcile(
        &mut directory,
        &mut config,
        &mut samples,
        |_| Err("disk full".into()),
        |_| panic!("must not save settings"),
    );
    assert!(result.error.is_some());
    assert!(result.blocked.contains(&samples[0].id));
    assert_eq!(serde_json::to_vec(&config).unwrap(), before_config);
    assert_eq!(serde_json::to_vec(&directory).unwrap(), before_directory);
    assert_eq!(config.threshold_for(&a), 0);
    assert!(config.device_alerts[&a].latched);
}
