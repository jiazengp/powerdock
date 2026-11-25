use super::*;
use std::time::Instant;

#[test]
fn expired_budget_cannot_start_io_even_with_idle_capacity() {
    let gate = Gate::default();
    let epoch = AtomicU64::new(1);
    let stopped = AtomicBool::new(false);
    let expired = Instant::now() - Duration::from_millis(1);
    assert!(
        matches!(gate.acquire("idle", &epoch, 1, &stopped, expired), Err(error) if error.starts_with("budget_deferred:"))
    );
    assert_eq!(gate.state.lock().unwrap().0, 0);
}

#[test]
fn deadline_is_rechecked_after_waiting_for_admission_lock() {
    let gate = Arc::new(Gate::default());
    let guard = gate.state.lock().unwrap();
    let waiting = gate.clone();
    let (entered, ready) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        let epoch = AtomicU64::new(1);
        let stopped = AtomicBool::new(false);
        let deadline = Instant::now() + Duration::from_millis(20);
        entered.send(deadline).unwrap();
        waiting
            .acquire("late", &epoch, 1, &stopped, deadline)
            .map(|_| ())
            .map_err(|e| Failure::from_error(&e))
    });
    let deadline = ready.recv_timeout(Duration::from_secs(2)).unwrap();
    thread::sleep(deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(5));
    drop(guard);
    assert_eq!(worker.join().unwrap(), Err(Failure::BudgetDeferred));
    assert_eq!(gate.state.lock().unwrap().0, 0);
}

#[test]
fn epoch_or_stop_cancels_an_entered_wait_before_capacity_is_released() {
    for revoke_epoch in [true, false] {
        let gate = Arc::new(Gate::default());
        let epoch = Arc::new(AtomicU64::new(1));
        let stopped = Arc::new(AtomicBool::new(false));
        let held = gate
            .acquire(
                "shared",
                &epoch,
                1,
                &stopped,
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        let waiting = gate.clone();
        let query_epoch = epoch.clone();
        let query_stop = stopped.clone();
        let (finished, result) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let outcome = waiting.acquire(
                "shared",
                &query_epoch,
                1,
                &query_stop,
                Instant::now() + Duration::from_secs(5),
            );
            finished
                .send(outcome.map(|_| ()).map_err(|e| Failure::from_error(&e)))
                .unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        while gate.queued_waiters() != 1 {
            assert!(
                Instant::now() < deadline,
                "query never entered admission wait"
            );
            thread::yield_now();
        }
        if revoke_epoch {
            epoch.fetch_add(1, Ordering::AcqRel);
        } else {
            stopped.store(true, Ordering::Release);
        }
        // Keep the receiver occupied until cancellation has completed.
        assert_eq!(
            result.recv_timeout(Duration::from_secs(2)).unwrap(),
            Err(Failure::Cancelled)
        );
        worker.join().unwrap();
        assert_eq!(gate.queued_waiters(), 0);
        assert_eq!(gate.state.lock().unwrap().0, 1);
        drop(held);
        assert_eq!(gate.state.lock().unwrap().0, 0);
    }
}
