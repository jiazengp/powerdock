use super::*;

#[test]
fn real_workers_cap_parallel_reads_and_release_capacity() {
    let targets: Vec<_> = (0..6)
        .map(|i| target(&format!("unit-{i}"), "synthetic"))
        .collect();
    let fixture = Fixture::new(&targets);
    let workers: Vec<_> = targets
        .iter()
        .enumerate()
        .map(|(i, target)| fixture.spawn(target.clone(), &format!("unit-{i}")))
        .collect();
    let mut pending: Vec<_> = (0..MAX_PARALLEL_QUERIES).map(|_| fixture.next()).collect();
    assert_eq!(
        fixture.activity.active.load(Ordering::Acquire),
        MAX_PARALLEL_QUERIES
    );
    assert!(
        fixture
            .started
            .recv_timeout(Duration::from_millis(30))
            .is_err()
    );
    let mut entered: Vec<_> = pending.iter().map(|read| read.id.clone()).collect();
    for _ in MAX_PARALLEL_QUERIES..targets.len() {
        pending.pop().unwrap().complete(42);
        let next = fixture.next();
        entered.push(next.id.clone());
        pending.push(next);
    }
    for read in pending {
        read.complete(42);
    }
    for target in &targets {
        fixture.expect_percent(&target.id(), 42);
    }
    entered.sort();
    entered.dedup();
    assert_eq!(entered.len(), targets.len());
    assert_eq!(
        fixture.activity.peak.load(Ordering::Acquire),
        MAX_PARALLEL_QUERIES
    );
    drop(workers);
    fixture.expect_idle();
}

#[test]
fn shared_receiver_waits_without_blocking_another_receiver() {
    let shared = |route| Target::Hid(hid_standard::Device::test_associated("shared", route));
    let first = shared("slot-1");
    let second = shared("slot-2");
    let other = target("other", "synthetic");
    let fixture = Fixture::new(&[first.clone(), second.clone(), other.clone()]);
    let a = fixture.spawn(first.clone(), "shared");
    let slow = fixture.next();
    assert_eq!(slow.id, first.id());
    let b = fixture.spawn(second.clone(), "shared");
    let c = fixture.spawn(other.clone(), "other");
    let fast = fixture.next();
    assert_eq!(fast.id, other.id());
    fast.complete(77);
    fixture.expect_percent(&other.id(), 77);
    assert!(
        fixture
            .started
            .recv_timeout(Duration::from_millis(30))
            .is_err()
    );
    slow.complete(42);
    let next = fixture.next();
    assert_eq!(next.id, second.id());
    next.complete(55);
    fixture.expect_percent(&first.id(), 42);
    fixture.expect_percent(&second.id(), 55);
    drop((a, b, c));
    fixture.expect_idle();
}

#[test]
fn slow_receiver_does_not_starve_other_workers_at_planned_capacities() {
    for capacity in [8, 16, 32] {
        let shared = |route| Target::Hid(hid_standard::Device::test_associated("shared", route));
        let blocked = shared("blocked");
        let sibling = shared("sibling");
        let mut targets = vec![blocked.clone(), sibling.clone()];
        targets.extend((2..capacity).map(|i| target(&format!("unit-{i}"), "synthetic")));
        let fixture = Fixture::new(&targets);
        let slow_worker = fixture.spawn(blocked.clone(), "shared");
        let slow = fixture.next();
        assert_eq!(slow.id, blocked.id());
        let sibling_worker = fixture.spawn(sibling.clone(), "shared");
        until(|| fixture.gate.queued_waiters() == 1);
        let workers: Vec<_> = targets[2..]
            .iter()
            .map(|target| fixture.spawn(target.clone(), &target.id()))
            .collect();
        let mut entered = std::collections::BTreeSet::new();
        for _ in &workers {
            let fast = fixture.next();
            assert_ne!(fast.id, sibling.id(), "shared receiver was still busy");
            assert_ne!(fast.id, blocked.id());
            assert!(entered.insert(fast.id.clone()), "duplicate query");
            fast.complete(42);
        }
        for target in &targets[2..] {
            fixture.expect_percent(&target.id(), 42);
        }
        slow_worker.cancel();
        slow.complete(10); // Simulate a driver result completing after revocation.
        let next = fixture.next();
        assert_eq!(next.id, sibling.id());
        next.complete(77);
        fixture.expect_percent(&sibling.id(), 77);
        drop((slow_worker, sibling_worker, workers));
        assert!(fixture.sample(&blocked.id()).result.is_err());
        assert!(fixture.activity.peak.load(Ordering::Acquire) <= MAX_PARALLEL_QUERIES);
        fixture.expect_idle();
    }
}

#[test]
fn cancelling_worker_waiting_on_receiver_never_enters_reader() {
    let t = target("occupied", "synthetic");
    let fixture = Fixture::new(std::slice::from_ref(&t));
    let epoch = AtomicU64::new(1);
    let stopped = AtomicBool::new(false);
    let occupied = fixture
        .gate
        .acquire("occupied", &epoch, 1, &stopped, Instant::now() + WAIT)
        .unwrap();
    let worker = fixture.spawn(t, "occupied");
    until(|| fixture.gate.queued_waiters() == 1);
    worker.cancel();
    drop(worker);
    assert_eq!(fixture.gate.queued_waiters(), 0);
    assert!(fixture.started.try_recv().is_err());
    drop(occupied);
    fixture.expect_idle();
}
