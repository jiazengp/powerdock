use super::*;

#[test]
fn full_wake_queue_does_not_lose_the_latest_interval() {
    let target = target("changing-interval", "synthetic");
    let fixture = Fixture::new(std::slice::from_ref(&target));
    let worker = fixture.spawn(target.clone(), "synthetic");
    let pending = fixture.next();
    worker.tx.try_send(()).unwrap();
    worker.tx.try_send(()).unwrap();
    assert!(matches!(
        worker.tx.try_send(()),
        Err(mpsc::TrySendError::Full(()))
    ));
    fixture.controls.set_interval(120);
    fixture.controls.set_interval(30);
    assert_eq!(fixture.controls.interval(), 30);
    pending.complete(42);
    fixture.next().complete(55);
    fixture.expect_percent(&target.id(), 55);
}

#[test]
fn hundred_real_worker_cycles_reject_late_reads_on_revoke_stop_and_topology_change() {
    let changing = target("replaceable", "synthetic");
    let unaffected = Target::Hid(hid_standard::Device::test_associated(
        "unaffected",
        "other-reader",
    ));
    let fixture = Fixture::new(&[changing.clone(), unaffected.clone()]);
    let other = fixture.spawn(unaffected.clone(), "unaffected");
    let other_read = fixture.next();
    let other_session = other_read.session;
    let other_epoch = other_read.epoch;
    other_read.complete(77);
    fixture.expect_percent(&unaffected.id(), 77);
    let enabled = fixture.controls.policy();
    for cycle in 0..100 {
        fixture.controls.apply(enabled.clone());
        let generation = fixture.publisher.generation.load(Ordering::Acquire);
        fixture.publisher.change(generation, |snapshot| {
            snapshot.reconcile(&[changing.clone(), unaffected.clone()]);
            *snapshot
                .devices
                .iter_mut()
                .find(|s| s.id == changing.id())
                .unwrap() = changing.sample();
        });
        let worker = fixture.spawn(changing.clone(), "replaceable");
        let late = fixture.next();
        assert_eq!(late.id, changing.id());
        assert_eq!(fixture.controls.0.lock().unwrap().workers.len(), 2);
        match cycle % 3 {
            0 => {
                // Hold the next query after wake-queue drain, so a fresh epoch
                // cannot overwrite a forbidden late publication before inspection.
                let metadata = worker.metadata.lock().unwrap();
                let mut disabled = enabled.clone();
                disabled
                    .disabled_readers
                    .insert(settings::Reader::MchoseHid);
                assert_eq!(fixture.controls.apply(disabled), vec![changing.id()]);
                assert!(!fixture.controls.accepts(&late.id, late.session, late.epoch));
                worker.tx.try_send(()).unwrap();
                late.complete(99);
                until(|| worker.tx.try_send(()).is_ok());
                assert!(fixture.sample(&changing.id()).result.is_err());
                drop(metadata);
                // The queued policy wake must publish only the new, disabled epoch.
                until(|| fixture.sample(&changing.id()).reader_epoch > 1);
                assert!(fixture.sample(&changing.id()).result.is_err());
            }
            1 => {
                worker.cancel();
                late.complete(99);
            }
            _ => {
                let next = generation + 1;
                fixture.publisher.generation.store(next, Ordering::Release);
                fixture.publisher.change(next, |snapshot| {
                    snapshot.reconcile(&[changing.clone(), unaffected.clone()])
                });
                late.complete(99);
                worker.tx.try_send(()).unwrap();
                let fresh = fixture.next();
                assert!(fixture.sample(&changing.id()).result.is_err());
                fresh.complete(42);
                fixture.expect_percent(&changing.id(), 42);
            }
        }
        drop(worker); // joins the real thread, including its in-flight completion.
        assert_ne!(
            fixture
                .sample(&changing.id())
                .result
                .as_ref()
                .ok()
                .and_then(|r| r.percent()),
            Some(99)
        );
        assert!(
            fixture
                .controls
                .accepts(&unaffected.id(), other_session, other_epoch)
        );
        // Topology publication rebuilds its snapshot; refresh the unchanged actor when needed.
        if cycle % 3 == 2 {
            other.tx.try_send(()).unwrap();
            let refreshed = fixture.next();
            assert_eq!(refreshed.id, unaffected.id());
            refreshed.complete(77);
            fixture.expect_percent(&unaffected.id(), 77);
        }
        assert_eq!(fixture.sample(&unaffected.id()).session, other_session);
    }
    drop(other);
    fixture.expect_idle();
}
