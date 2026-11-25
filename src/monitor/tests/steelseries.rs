use super::*;
use crate::adapters::steelseries::tests::interface;

#[test]
fn steelseries_os_fallbacks_share_one_actor_before_budget_admission() {
    let mut targets = vec![Target::Windows(system::Peripheral::test_paired(
        "unit", true,
    ))];
    attach_standard_hid(
        &mut targets,
        vec![hid_standard::Device::test_associated("unit", "standard")],
    );
    attach_steelseries(&mut targets, vec![interface("unit", 0x1890)]);
    assert_eq!(targets.len(), 1);
    let Target::SteelSeries(target) = &targets[0] else {
        panic!("not attached")
    };
    assert!(target.windows.is_some());
    // The standard reader lives in the OS fallback when already associated.
    assert!(target.windows.as_ref().unwrap().standard_hid.is_some());
    let sources = targets[0].sources();
    assert!(sources.contains(&Source::SteelSeries));
    assert!(sources.contains(&Source::Windows));
    assert!(sources.contains(&Source::HidStandard));
    let catalog = prioritize_targets(&mut targets, &settings::ProviderPreferences::default());
    assert_eq!(catalog.len(), 1);
    assert!(metadata_wake(&targets[0], &None));
}

#[test]
fn steelseries_receiver_identity_stays_route_only_and_hotplug_is_exact() {
    for pid in [0x1890, 0x1880, 0x1892] {
        let d = interface("unit", pid);
        let mut targets = Vec::new();
        attach_steelseries(&mut targets, vec![d.clone()]);
        let sample = targets[0].sample();
        assert_eq!(sample.source, Source::SteelSeries);
        assert_eq!(sample.kind, Kind::Mouse);
        if pid == 0x1892 {
            assert!(matches!(sample.identity, Evidence::Strong(_)));
        } else {
            assert!(matches!(sample.identity, Evidence::Route(_)));
        }
        let path = String::from_utf16_lossy(&d.path[..d.path.len() - 1]);
        assert!(targets[0].matches_interface(&[path.to_uppercase()]));
        assert!(!targets[0].matches_interface(&["unrelated".into()]));
    }
}

#[test]
fn disabling_steelseries_revokes_only_its_epoch_and_preserves_standard_sources() {
    let mut enabled = settings::ProviderPreferences::default();
    enabled
        .enabled_experimental
        .insert(settings::Profile::SteelseriesDirectExperimental);
    let controls = Controls::new(enabled.clone());
    let mut targets = Vec::new();
    attach_steelseries(&mut targets, vec![interface("steel", 0x1890)]);
    let steel = targets.remove(0);
    let mouse = target("mchose", "synthetic");
    let mut retained = Vec::new();
    for (session, target) in [(1, steel.clone()), (2, mouse)] {
        let epoch = Arc::new(AtomicU64::new(1));
        let (wake, rx) = mpsc::sync_channel(2);
        controls.0.lock().unwrap().workers.push(Registration {
            id: target.id(),
            session,
            epoch: Arc::downgrade(&epoch),
            target,
            wake,
        });
        retained.push((epoch, rx));
    }
    enabled
        .disabled_readers
        .insert(settings::Reader::SteelseriesHid);
    assert_eq!(controls.apply(enabled.clone()), vec![steel.id()]);
    assert_eq!(retained[0].0.load(Ordering::Acquire), 2);
    assert!(retained[0].1.try_recv().is_ok());
    assert_eq!(retained[1].0.load(Ordering::Acquire), 1);
    assert!(retained[1].1.try_recv().is_err());
    assert!(enabled.source(Source::Windows));
    assert!(enabled.source(Source::Mchose));
    assert!(controls.apply(enabled).is_empty());
}

#[test]
fn disabled_steelseries_is_rejected_at_actual_reader_boundary() {
    let mut targets = Vec::new();
    attach_steelseries(&mut targets, vec![interface("never-open", 0x1890)]);
    let target = targets.remove(0);
    let mut reader = crate::monitor::reader::Reader::new();
    let mut sample = target.sample();
    let result = reader.read(
        &target,
        &mut sample,
        &AtomicU64::new(1),
        1,
        &settings::ProviderPreferences::default(),
    );
    assert!(
        result
            .unwrap_err()
            .starts_with("unsupported: SteelSeries reader disabled")
    );
}
