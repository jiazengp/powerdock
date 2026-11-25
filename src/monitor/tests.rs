#[path = "tests/discovery.rs"]
mod candidate_tests;
#[path = "tests/steelseries.rs"]
mod steelseries_tests;

use super::discovery::*;

#[test]
fn hundred_topology_and_policy_cycles_fence_late_work_and_release_budget() {
    let generation = Arc::new(AtomicU64::new(1));
    let latest = Arc::new(Mutex::new(None));
    let publisher = Publisher {
        latest: latest.clone(),
        generation: generation.clone(),
        host: 0,
        message: 0,
    };
    let controls = Controls::new(settings::ProviderPreferences::default());
    let gate = Gate::default();
    let slow_epoch = AtomicU64::new(1);
    let stopped = AtomicBool::new(false);
    let slow = gate
        .acquire(
            "slow-receiver",
            &slow_epoch,
            1,
            &stopped,
            std::time::Instant::now() + Duration::from_millis(100),
        )
        .unwrap();
    for cycle in 1..=100 {
        generation.store(cycle, Ordering::Release);
        let current = target(&format!("unit-{cycle}"), "fast-receiver");
        let id = current.id();
        publisher.change(cycle, |state| {
            state.reconcile(std::slice::from_ref(&current))
        });
        let epoch = Arc::new(AtomicU64::new(1));
        let (tx, rx) = mpsc::sync_channel(2);
        controls.0.lock().unwrap().workers.push(Registration {
            id: id.clone(),
            session: cycle,
            epoch: Arc::downgrade(&epoch),
            target: current,
            wake: tx,
        });
        let fast = gate
            .acquire(
                "fast-receiver",
                &epoch,
                1,
                &stopped,
                std::time::Instant::now() + Duration::from_millis(100),
            )
            .unwrap();
        let mut disabled = controls.policy();
        disabled
            .disabled_readers
            .insert(settings::Reader::MchoseHid);
        assert_eq!(controls.apply(disabled), vec![id.clone()]);
        assert!(!controls.accepts(&id, cycle, 1));
        assert!(rx.try_recv().is_ok());
        assert!(
            gate.acquire(
                "fast-receiver",
                &epoch,
                1,
                &stopped,
                std::time::Instant::now() + Duration::from_millis(100)
            )
            .is_err()
        );
        publisher.change(cycle - 1, |state| state.error = Some("late result".into()));
        let snapshot = latest.lock().unwrap().clone().unwrap();
        assert_eq!(snapshot.devices.len(), 1);
        assert_eq!(snapshot.devices[0].id, id);
        assert!(snapshot.error.is_none());
        drop(fast);
        controls.apply(settings::ProviderPreferences::default());
        assert_eq!(controls.0.lock().unwrap().workers.len(), 1);
    }
    drop(slow);
    let permits: Vec<_> = (0..MAX_PARALLEL_QUERIES)
        .map(|i| {
            gate.acquire(
                &format!("receiver-{i}"),
                &slow_epoch,
                1,
                &stopped,
                std::time::Instant::now() + Duration::from_millis(100),
            )
            .unwrap()
        })
        .collect();
    assert!(
        gate.acquire(
            "overflow",
            &slow_epoch,
            1,
            &stopped,
            std::time::Instant::now() + Duration::from_millis(100)
        )
        .is_err()
    );
    drop(permits);
}
use super::*;
#[test]
fn discovery_catalog_retains_over_budget_candidates_across_repeated_refreshes() {
    let original: Vec<_> = (0..40)
        .map(|i| target(&format!("unit-{i:02}"), &format!("path-{i}")))
        .collect();
    let mut expected = None;
    for _ in 0..100 {
        let mut targets = original.clone();
        targets.push(original[0].clone());
        let catalog = prioritize_targets(&mut targets, &settings::ProviderPreferences::default());
        assert_eq!(targets.len(), battery::MAX_DEVICES);
        assert_eq!(catalog.len(), 40);
        assert_eq!(
            catalog.iter().filter(|c| c.scheduling == "active").count(),
            battery::MAX_DEVICES
        );
        assert_eq!(
            catalog
                .iter()
                .filter(|c| c.scheduling == "budget_deferred")
                .count(),
            40 - battery::MAX_DEVICES
        );
        assert_eq!(
            catalog
                .iter()
                .take(battery::MAX_DEVICES)
                .map(|c| c.id.clone())
                .collect::<Vec<_>>(),
            targets.iter().map(Target::id).collect::<Vec<_>>()
        );
        if let Some(expected) = &expected {
            assert_eq!(&catalog, expected);
        }
        expected = Some(catalog);
    }
}
#[test]
fn io_budget_caps_parallel_queries_and_serializes_shared_receivers() {
    let gate = Gate::default();
    let generation = AtomicU64::new(1);
    let stopped = AtomicBool::new(false);
    let acquire = |id| {
        gate.acquire(
            id,
            &generation,
            1,
            &stopped,
            std::time::Instant::now() + Duration::from_millis(100),
        )
    };
    let slow = acquire("slow").unwrap();
    let fast = acquire("fast").unwrap();
    assert!(acquire("slow").is_err());
    let a = acquire("a").unwrap();
    let b = acquire("b").unwrap();
    assert!(acquire("fifth").is_err());
    drop(fast);
    assert!(acquire("next").is_ok());
    drop((slow, a, b));
    assert!(acquire("slow").is_ok());
    generation.store(2, Ordering::Release);
    assert!(acquire("cancelled").is_err());
}
#[test]
fn topology_debounce_and_periodic_repair_use_deadlines_without_idle_spin() {
    let t = std::time::Instant::now();
    let mut clock = ScanClock::new(t);
    assert!(clock.due(t));
    clock.scanned(t);
    clock.event(t + Duration::from_secs(1));
    clock.event(t + Duration::from_millis(1100));
    assert!(!clock.due(t + Duration::from_millis(1299)));
    assert!(clock.due(t + Duration::from_millis(1300)));
    clock.scanned(t + Duration::from_millis(1300));
    assert_eq!(
        clock.wait(t + Duration::from_millis(1300)),
        Duration::from_secs(300)
    );
    assert!(clock.due(t + Duration::from_millis(301300)));
}
#[test]
fn typed_retry_classes_bound_failures_and_keep_unknown_readings_out_of_retry_storms() {
    assert_eq!(Failure::from_error(mchose::a7::BUSY), Failure::Busy);
    assert_eq!(
        Failure::from_error(mchose::keyboard::WAITING),
        Failure::SleepSuspected
    );
    assert_eq!(next_poll(Some(Failure::Busy), 60, 5, false), 30);
    assert_eq!(next_poll(Some(Failure::AccessDenied), 30, 1, false), 300);
    assert_eq!(next_poll(None, 30, 0, true), 300);
    for count in 1..10 {
        assert!((60..=300).contains(&next_poll(Some(Failure::Timeout), 30, count, false)));
    }
}
#[test]
fn configure_and_device_refresh_batches_always_require_discovery_in_either_order() {
    for configure_first in [true, false] {
        let (tx, rx) = mpsc::sync_channel(4);
        let configure = Command::Configure;
        let refresh = Command::RefreshDevice {
            ids: vec!["existing".into()],
        };
        let (first, second) = if configure_first {
            (configure, refresh)
        } else {
            (refresh, configure)
        };
        tx.send(first).unwrap();
        tx.send(second).unwrap();
        let batch = Batch::collect(Some(rx.recv().unwrap()), &rx);
        assert!(batch.needs_discovery());
        assert_eq!(batch.requested, vec!["existing"]);
    }
    let (tx, rx) = mpsc::sync_channel(4);
    tx.send(Command::RefreshDevice {
        ids: vec!["existing".into()],
    })
    .unwrap();
    assert!(!Batch::collect(Some(rx.recv().unwrap()), &rx).needs_discovery());
}
#[test]
fn mouse_profile_changes_preserve_other_profiles_and_cancel_only_their_own_epoch() {
    use settings::Profile;
    let controls = Controls::new(settings::ProviderPreferences::default());
    let a7 = target("a7", "a7");
    let mut yjx = target("yjx", "yjx");
    if let Target::Mchose(d) = &mut yjx {
        d.product_id = 0x4245;
        d.usage = 0x10;
        d.input_bytes = 65;
        d.output_bytes = 65;
    }
    let a = Arc::new(AtomicU64::new(1));
    let b = Arc::new(AtomicU64::new(1));
    let (a_tx, a_rx) = mpsc::sync_channel(2);
    let (b_tx, b_rx) = mpsc::sync_channel(2);
    controls.0.lock().unwrap().workers = vec![
        Registration {
            id: a7.id(),
            session: 1,
            epoch: Arc::downgrade(&a),
            target: a7,
            wake: a_tx,
        },
        Registration {
            id: yjx.id(),
            session: 2,
            epoch: Arc::downgrade(&b),
            target: yjx,
            wake: b_tx,
        },
    ];
    let mut policy = controls.policy();
    policy
        .enabled_experimental
        .insert(Profile::MchoseYjxExperimental);
    let affected = controls.apply(policy.clone());
    assert_eq!(affected.len(), 1);
    assert_eq!(affected[0], controls.0.lock().unwrap().workers[1].id);
    assert_eq!(a.load(Ordering::Acquire), 1);
    assert!(a_rx.try_recv().is_err());
    assert_eq!(b.load(Ordering::Acquire), 2);
    assert!(b_rx.try_recv().is_ok());
    policy.disabled_profiles.insert(Profile::MchoseA7);
    let affected = controls.apply(policy);
    assert_eq!(affected.len(), 1);
    assert_eq!(affected[0], controls.0.lock().unwrap().workers[0].id);
    assert_eq!(a.load(Ordering::Acquire), 2);
    assert_eq!(b.load(Ordering::Acquire), 2);
    assert!(b_rx.try_recv().is_err());
}
#[test]
fn provider_changes_cancel_only_affected_inflight_work_and_do_not_apply_on_noop() {
    let controls = Controls::new(settings::ProviderPreferences::default());
    let a = Arc::new(AtomicU64::new(1));
    let b = Arc::new(AtomicU64::new(1));
    let (a_tx, a_rx) = mpsc::sync_channel(2);
    let (b_tx, b_rx) = mpsc::sync_channel(2);
    controls.0.lock().unwrap().workers = vec![
        Registration {
            id: "headset".into(),
            session: 1,
            epoch: Arc::downgrade(&a),
            target: Target::Windows(system::Peripheral::test_classic_headset("headset")),
            wake: a_tx,
        },
        Registration {
            id: "mouse".into(),
            session: 2,
            epoch: Arc::downgrade(&b),
            target: target("mouse", "mouse"),
            wake: b_tx,
        },
    ];
    let mut policy = controls.policy();
    policy
        .disabled_readers
        .insert(settings::Reader::SennheiserGaia);
    let inflight = a.load(Ordering::Acquire);
    assert!(controls.accepts("headset", 1, inflight));
    controls.apply(policy.clone());
    assert_ne!(a.load(Ordering::Acquire), inflight);
    assert_eq!(b.load(Ordering::Acquire), 1);
    assert_eq!(a_rx.try_recv(), Ok(()));
    assert!(b_rx.try_recv().is_err());
    assert_eq!(controls.generation(), 2);
    assert!(!controls.accepts("headset", 1, inflight));
    assert!(!controls.accepts("headset", 9, inflight + 1));
    assert!(controls.accepts("mouse", 2, 1));
    controls.apply(policy);
    assert_eq!(controls.generation(), 2);
    assert!(controls.policy().source(Source::Windows));
    assert!(controls.policy().source(Source::BluetoothGatt));
}
pub(super) fn target(identity: &str, path: &str) -> Target {
    Target::Mchose(device::Device {
        path: path.encode_utf16().chain(Some(0)).collect(),
        container: Some(identity.into()),
        vendor_id: 0x3837,
        product_id: 0x1018,
        product: identity.into(),
        usage_page: 0xff01,
        usage: 1,
        input_bytes: 64,
        output_bytes: 64,
    })
}
