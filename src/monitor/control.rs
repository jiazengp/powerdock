//! Effective-policy revocation and shared I/O admission.
use super::*;

#[cfg(test)]
mod tests;

#[derive(Clone)]
pub struct Controls(pub(super) Arc<Mutex<ControlState>>, Arc<AtomicU64>);
pub(super) struct Registration {
    pub(super) id: String,
    pub(super) session: u64,
    pub(super) epoch: std::sync::Weak<AtomicU64>,
    pub(super) target: Target,
    pub(super) wake: SyncSender<()>,
}
pub(super) struct ControlState {
    pub(super) policy: settings::ProviderPreferences,
    pub(super) generation: u64,
    pub(super) workers: Vec<Registration>,
}
impl Controls {
    pub fn accepts(&self, id: &str, session: u64, expected: u64) -> bool {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.workers.iter().any(|w| {
            w.id == id
                && w.session == session
                && w.epoch
                    .upgrade()
                    .is_some_and(|epoch| epoch.load(Ordering::Acquire) == expected)
        })
    }
    pub fn new(policy: settings::ProviderPreferences) -> Self {
        Self(
            Arc::new(Mutex::new(ControlState {
                policy,
                generation: 1,
                workers: vec![],
            })),
            Arc::new(AtomicU64::new(settings::Settings::default().interval_secs)),
        )
    }
    pub fn set_interval(&self, interval: u64) {
        if self.1.swap(interval, Ordering::AcqRel) != interval {
            let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
            for worker in &state.workers {
                let _ = worker.wake.try_send(());
            }
        }
    }
    pub(super) fn interval(&self) -> u64 {
        self.1.load(Ordering::Acquire)
    }
    pub fn policy(&self) -> settings::ProviderPreferences {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .policy
            .clone()
    }
    pub fn generation(&self) -> u64 {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).generation
    }
    /// Caller must persist the validated settings first. Only affected sessions cancel.
    pub fn apply(&self, next: settings::ProviderPreferences) -> Vec<String> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.policy == next {
            return Vec::new();
        }
        let mut affected = Vec::new();
        state.workers.retain(|w| w.epoch.strong_count() > 0);
        for w in &state.workers {
            if w.target.policy_changed(&state.policy, &next)
                && let Some(epoch) = w.epoch.upgrade()
            {
                affected.push(w.id.clone());
                epoch.fetch_add(1, Ordering::AcqRel);
                let _ = w.wake.try_send(());
            }
        }
        state.policy = next;
        state.generation += 1;
        affected
    }
}

#[derive(Default)]
pub(super) struct Gate {
    state: Mutex<(usize, std::collections::BTreeSet<String>)>,
    changed: std::sync::Condvar,
    #[cfg(test)]
    waiting: std::sync::atomic::AtomicUsize,
}
#[cfg(test)]
struct Waiting<'a>(&'a std::sync::atomic::AtomicUsize);
#[cfg(test)]
impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}
pub(super) struct Permit<'a> {
    gate: &'a Gate,
    key: String,
}
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.gate.state.lock() {
            state.0 -= 1;
            state.1.remove(&self.key);
            self.gate.changed.notify_all();
        }
    }
}
impl Gate {
    #[cfg(test)]
    pub(super) fn queued_waiters(&self) -> usize {
        // A waiter increments while holding state. Obtaining this same lock
        // proves it has released state inside Condvar::wait_timeout.
        let _state = self.state.lock().unwrap();
        self.waiting.load(Ordering::Acquire)
    }
    pub(super) fn acquire(
        &self,
        key: &str,
        generation: &AtomicU64,
        expected: u64,
        stopped: &AtomicBool,
        deadline: std::time::Instant,
    ) -> Result<Permit<'_>, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "budget_deferred: I/O budget unavailable")?;
        loop {
            if stopped.load(Ordering::Acquire) || generation.load(Ordering::Acquire) != expected {
                return Err("cancelled: I/O budget".into());
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Err("budget_deferred: I/O capacity reached".into());
            }
            if state.0 < MAX_PARALLEL_QUERIES && !state.1.contains(key) {
                state.0 += 1;
                state.1.insert(key.into());
                return Ok(Permit {
                    gate: self,
                    key: key.into(),
                });
            }
            #[cfg(test)]
            let _waiting = {
                self.waiting.fetch_add(1, Ordering::Release);
                Waiting(&self.waiting)
            };
            state = self
                .changed
                .wait_timeout(state, left.min(Duration::from_millis(50)))
                .map_err(|_| "budget_deferred: I/O wait")?
                .0;
        }
    }
}
