//! Script only the hardware boundary; use the real actor, gate and publisher.
mod admission;
mod lifecycle;

use super::*;
use crate::monitor::topology_tests::target;
use std::{collections::BTreeMap, sync::atomic::AtomicUsize, time::Instant};

const WAIT: Duration = Duration::from_secs(5);

struct Started {
    id: String,
    session: u64,
    epoch: u64,
    reply: SyncSender<u8>,
}
impl Started {
    fn complete(self, percent: u8) {
        self.reply.send(percent).unwrap();
    }
}

#[derive(Default)]
struct Activity {
    active: AtomicUsize,
    peak: AtomicUsize,
    receivers: Mutex<BTreeMap<String, usize>>,
}
struct ActiveRead(Arc<Activity>, String);
impl Drop for ActiveRead {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::AcqRel);
        *self.0.receivers.lock().unwrap().get_mut(&self.1).unwrap() -= 1;
    }
}

struct Fixture {
    publisher: Publisher,
    controls: Controls,
    gate: Arc<Gate>,
    activity: Arc<Activity>,
    events: mpsc::Sender<Started>,
    started: Receiver<Started>,
}
impl Fixture {
    fn new(targets: &[Target]) -> Self {
        let (events, started) = mpsc::channel();
        let mut policy = settings::ProviderPreferences::default();
        policy
            .enabled_experimental
            .insert(settings::Profile::HidStandardExperimental);
        let fixture = Self {
            publisher: Publisher {
                latest: Arc::new(Mutex::new(None)),
                generation: Arc::new(AtomicU64::new(1)),
                host: 0,
                message: 0,
            },
            controls: Controls::new(policy),
            gate: Arc::default(),
            activity: Arc::default(),
            events,
            started,
        };
        fixture
            .publisher
            .change(1, |snapshot| snapshot.reconcile(targets));
        fixture
    }
    fn spawn(&self, target: Target, receiver: &str) -> Worker {
        let events = self.events.clone();
        let activity = self.activity.clone();
        let receiver = receiver.to_owned();
        Worker::start(
            target,
            self.publisher.clone(),
            self.controls.clone(),
            self.gate.clone(),
            None,
            move || {
                Reader::scripted(move |sample, _, expected| {
                    let active = activity.active.fetch_add(1, Ordering::AcqRel) + 1;
                    activity.peak.fetch_max(active, Ordering::AcqRel);
                    let _reading = ActiveRead(activity.clone(), receiver.clone());
                    {
                        let mut receivers = activity.receivers.lock().unwrap();
                        let count = receivers.entry(receiver.clone()).or_default();
                        *count += 1;
                        assert_eq!(*count, 1, "concurrent reads on one receiver");
                    }
                    let (reply, result) = mpsc::sync_channel(1);
                    events
                        .send(Started {
                            id: sample.id.clone(),
                            session: sample.session,
                            epoch: expected,
                            reply,
                        })
                        .unwrap();
                    // Intentionally ignore cancellation here: a driver can complete late.
                    let percent = result
                        .recv_timeout(WAIT)
                        .map_err(|_| "timeout: scripted read")?;
                    Ok(battery::Reading::new(
                        battery::Value::reported(percent),
                        Kind::Mouse,
                        battery::Power::Battery,
                    ))
                })
            },
        )
    }
    fn next(&self) -> Started {
        self.started
            .recv_timeout(WAIT)
            .expect("worker did not enter its read")
    }
    fn sample(&self, id: &str) -> Sample {
        self.publisher
            .latest
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .devices
            .iter()
            .find(|s| s.id == id)
            .unwrap()
            .clone()
    }
    fn expect_percent(&self, id: &str, percent: u8) {
        until(|| {
            self.sample(id)
                .result
                .as_ref()
                .is_ok_and(|reading| reading.percent() == Some(percent))
        });
    }
    fn expect_idle(&self) {
        assert_eq!(self.activity.active.load(Ordering::Acquire), 0);
        assert!(
            self.controls
                .0
                .lock()
                .unwrap()
                .workers
                .iter()
                .all(|w| w.epoch.strong_count() == 0)
        );
        let epoch = AtomicU64::new(1);
        let stopped = AtomicBool::new(false);
        let permits: Vec<_> = (0..MAX_PARALLEL_QUERIES)
            .map(|i| {
                self.gate
                    .acquire(
                        &format!("check-{i}"),
                        &epoch,
                        1,
                        &stopped,
                        Instant::now() + WAIT,
                    )
                    .unwrap()
            })
            .collect();
        drop(permits);
    }
}
fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "worker did not reach expected state"
        );
        thread::sleep(Duration::from_millis(1));
    }
}
