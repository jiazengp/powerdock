//! One target actor: sessions, observations and bounded retry scheduling.
use super::reader::Reader;
use super::*;

#[cfg(test)]
mod tests;

pub(super) fn transfer_link_observation(sample: &mut Sample, link: &system::Link) {
    if sample.result.is_ok()
        && matches!(
            sample.source,
            Source::BluetoothGatt | Source::Windows | Source::Sennheiser
        )
    {
        sample.components = link.components.clone();
        sample.value_observation = link.observation;
    }
}

pub(super) struct Worker {
    pub(super) metadata: Arc<Mutex<Option<system::MetadataBattery>>>,
    pub(super) id: String,
    pub(super) target: Target,
    pub(super) tx: SyncSender<()>,
    force_uncached: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    epoch: Arc<AtomicU64>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Worker {
    pub(super) fn refresh(&self) {
        // Keep refresh intent even when a wake is already queued.
        self.force_uncached.store(true, Ordering::Release);
        let _ = self.tx.try_send(());
    }
    pub(super) fn cancel(&self) {
        self.stop.store(true, Ordering::Release);
        self.epoch.fetch_add(1, Ordering::AcqRel);
        let _ = self.tx.try_send(());
    }
    pub(super) fn new(
        target: Target,
        publisher: Publisher,
        controls: Controls,
        gate: Arc<Gate>,
        initial_metadata: Option<system::MetadataBattery>,
    ) -> Self {
        Self::start(
            target,
            publisher,
            controls,
            gate,
            initial_metadata,
            Reader::new,
        )
    }
    fn start(
        target: Target,
        publisher: Publisher,
        controls: Controls,
        gate: Arc<Gate>,
        initial_metadata: Option<system::MetadataBattery>,
        initialize: impl FnOnce() -> Reader + Send + 'static,
    ) -> Self {
        let metadata = Arc::new(Mutex::new(initial_metadata));
        let query_metadata = metadata.clone();
        let id = target.id();
        let (tx, rx) = mpsc::sync_channel(2);
        let force_uncached = Arc::new(AtomicBool::new(false));
        let requested_uncached = force_uncached.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let identity = target.clone();
        let epoch = Arc::new(AtomicU64::new(1));
        static SESSIONS: AtomicU64 = AtomicU64::new(1);
        let session_id = SESSIONS.fetch_add(1, Ordering::Relaxed);
        {
            let mut state = controls.0.lock().unwrap_or_else(|e| e.into_inner());
            state.workers.retain(|w| w.epoch.strong_count() > 0);
            state.workers.push(Registration {
                id: id.clone(),
                session: session_id,
                epoch: Arc::downgrade(&epoch),
                target: target.clone(),
                wake: tx.clone(),
            });
        }
        let query_epoch = epoch.clone();
        let wake_tx = tx.clone();
        let wake: gatt::Wake = Arc::new(move || {
            let _ = wake_tx.try_send(());
        });
        let gate_key = match &target {
            Target::SteelSeries(t) => t.device.container.clone().unwrap_or_else(|| target.id()),
            Target::Mchose(d) | Target::Keyboard(d) => {
                d.container.clone().unwrap_or_else(|| target.id())
            }
            Target::Hidpp(t) => t
                .device
                .container
                .clone()
                .unwrap_or_else(|| t.device.gate_key()),
            Target::Hid(d) => d.container.clone().unwrap_or_else(|| target.id()),
            Target::Windows(d) => d.container.clone().unwrap_or_else(|| target.id()),
        };
        let thread = thread::spawn(move || {
            let mut reader = initialize();
            let mut timeout = Duration::ZERO;
            let mut failures = 0u64;
            let mut last_epoch = 1;
            while !matches!(
                rx.recv_timeout(timeout),
                Err(mpsc::RecvTimeoutError::Disconnected)
            ) {
                while rx.try_recv().is_ok() {}
                if stopped.load(Ordering::Acquire) {
                    break;
                }
                let expected = publisher.generation.load(Ordering::Acquire);
                if expected == u64::MAX {
                    break;
                }
                let mut sample = target.sample();
                sample.session = session_id;
                let (policy, query_expected) = {
                    let state = controls.0.lock().unwrap_or_else(|e| e.into_inner());
                    (state.policy.clone(), query_epoch.load(Ordering::Acquire))
                };
                sample.reader_epoch = query_expected;
                if last_epoch != query_expected {
                    reader.reset();
                    failures = 0;
                    last_epoch = query_expected;
                }
                reader.link.wake = Some(wake.clone());
                reader.link.metadata = query_metadata.lock().ok().and_then(|m| m.clone());
                reader.link.force_uncached = requested_uncached.swap(false, Ordering::AcqRel);
                let query_started = std::time::Instant::now();
                let permit = gate.acquire(
                    &gate_key,
                    &query_epoch,
                    query_expected,
                    &stopped,
                    query_started + Duration::from_secs(2),
                );
                sample.result = if let Err(error) = &permit {
                    Err(error.clone())
                } else {
                    reader.read(&target, &mut sample, &query_epoch, query_expected, &policy)
                };
                drop(permit);
                sample.at = settings::now();
                sample.clock = std::time::Instant::now();
                transfer_link_observation(&mut sample, &reader.link);
                if tracing::enabled!(tracing::Level::DEBUG) {
                    crate::diagnostics::debug(
                        "device_query",
                        serde_json::json!({"id":sample.id,"source":sample.source,"elapsed_ms":query_started.elapsed().as_millis(),"result":sample.result,"failures":failures}),
                    );
                }
                let failure = sample.result.as_ref().err().map(|e| Failure::from_error(e));
                if failure.is_some_and(|e| !e.soft()) {
                    failures = (failures + 1).min(5);
                    reader.clear_sessions();
                } else {
                    failures = 0;
                }
                // Serialize publication against configuration changes. An old source
                // cannot publish between cancellation and removal of its qualification.
                let control_state = controls.0.lock().unwrap_or_else(|e| e.into_inner());
                if query_epoch.load(Ordering::Acquire) != query_expected
                    || stopped.load(Ordering::Acquire)
                {
                    continue;
                }
                publisher.change(expected, |state| {
                    if let Some(entry) = state.devices.iter_mut().find(|d| d.id == sample.id) {
                        *entry = sample;
                    }
                });
                drop(control_state);
                timeout = Duration::from_secs(next_poll(
                    failure,
                    controls.interval(),
                    failures,
                    reader.link.subscribed,
                ));
            }
        });
        Self {
            metadata,
            id,
            target: identity,
            tx,
            force_uncached,
            stop,
            epoch,
            thread: Some(thread),
        }
    }
}
