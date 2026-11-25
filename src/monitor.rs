//! Monitoring coordinator: discoveries become bounded, independently cancellable actors.
mod control;
mod discovery;
mod reader;
#[cfg(test)]
#[path = "monitor/tests.rs"]
mod topology_tests;
mod worker;

pub use control::Controls;
use control::{Gate, Registration};
pub use discovery::Candidate;
use discovery::{Discovery, Target, discover_with};
use worker::Worker;

pub const MAX_PARALLEL_QUERIES: usize = 4;

use crate::{
    adapters::{device, gatt, hid_standard, hidpp, mchose, steelseries, system},
    battery::{self, Kind, Sample, Source},
    identity::Evidence,
    settings,
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::Duration,
};
use windows_sys::Win32::{Foundation::HWND, UI::WindowsAndMessaging::PostMessageW};

pub enum Command {
    DiscoveryChanged,
    Configure,
    Refresh { invalidate: bool },
    RefreshDevice { ids: Vec<String> },
    TopologyChanged { interfaces: Vec<String> },
    Stop,
}
#[derive(Clone)]
pub struct Snapshot {
    pub generation: u64,
    pub devices: Vec<Sample>,
    pub catalog: Vec<Candidate>,
    pub error: Option<String>,
}

impl Snapshot {
    fn reconcile(&mut self, targets: &[Target]) {
        self.error = None;
        let ids: Vec<_> = targets.iter().map(Target::id).collect();
        self.devices.retain(|sample| ids.contains(&sample.id));
        for target in targets {
            if !self.devices.iter().any(|sample| sample.id == target.id()) {
                self.devices.push(target.sample());
            }
        }
        self.devices.sort_by(|a, b| a.id.cmp(&b.id));
    }
}

pub fn list() -> Result<Vec<Sample>, String> {
    Ok(
        discover_with(&settings::ProviderPreferences::default(), None)?
            .targets
            .iter()
            .map(Target::sample)
            .collect(),
    )
}

type Latest = Arc<Mutex<Option<Snapshot>>>;
#[derive(Clone)]
struct Publisher {
    latest: Latest,
    generation: Arc<AtomicU64>,
    host: usize,
    message: u32,
}
impl Publisher {
    fn change(&self, expected: u64, update: impl FnOnce(&mut Snapshot)) {
        if self.generation.load(Ordering::Acquire) != expected {
            return;
        }
        if let Ok(mut slot) = self.latest.lock() {
            if self.generation.load(Ordering::Acquire) != expected {
                return;
            }
            let snapshot = slot.get_or_insert_with(|| Snapshot {
                generation: expected,
                devices: Vec::new(),
                catalog: Vec::new(),
                error: None,
            });
            if snapshot.generation != expected {
                *snapshot = Snapshot {
                    generation: expected,
                    devices: Vec::new(),
                    catalog: Vec::new(),
                    error: None,
                };
            }
            update(snapshot);
        }
        if self.host != 0 {
            unsafe {
                PostMessageW(self.host as HWND, self.message, 0, 0);
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    Busy,
    SleepSuspected,
    Timeout,
    AccessDenied,
    Unsupported,
    Cancelled,
    BudgetDeferred,
    Disconnected,
    InvalidReply,
    ReadFailed,
}
impl Failure {
    pub fn from_error(error: &str) -> Self {
        if error == mchose::a7::BUSY {
            return Self::Busy;
        }
        if error == mchose::keyboard::WAITING {
            return Self::SleepSuspected;
        }
        match error.split(':').next().unwrap_or_default() {
            "busy" => Self::Busy,
            "timeout" => Self::Timeout,
            "access_denied" => Self::AccessDenied,
            "unsupported" => Self::Unsupported,
            "cancelled" => Self::Cancelled,
            "budget_deferred" => Self::BudgetDeferred,
            "disconnected" => Self::Disconnected,
            "invalid_reply" => Self::InvalidReply,
            _ => Self::ReadFailed,
        }
    }
    fn soft(self) -> bool {
        matches!(
            self,
            Self::Busy | Self::SleepSuspected | Self::BudgetDeferred
        )
    }
}
fn next_poll(error: Option<Failure>, interval: u64, failures: u64, subscribed: bool) -> u64 {
    match error {
        Some(e) if e.soft() => interval.min(30),
        Some(Failure::AccessDenied | Failure::Unsupported) => 300,
        Some(_) => interval.max(60).saturating_mul(failures.max(1)).min(300),
        None if subscribed => 300,
        None => interval,
    }
    .min(300)
}
#[derive(Default)]
struct Batch {
    refresh: bool,
    invalidate: bool,
    configure: bool,
    discovery: bool,
    stop: bool,
    requested: Vec<String>,
    interfaces: Vec<String>,
}
impl Batch {
    fn collect(first: Option<Command>, rx: &Receiver<Command>) -> Self {
        let mut batch = Self::default();
        for command in first.into_iter().chain(rx.try_iter()) {
            match command {
                Command::Stop => batch.stop = true,
                Command::Configure => batch.configure = true,
                Command::DiscoveryChanged => batch.discovery = true,
                Command::RefreshDevice { ids } => batch.requested.extend(ids),
                Command::TopologyChanged { interfaces } => {
                    batch.interfaces.extend(interfaces);
                }
                Command::Refresh { invalidate } => {
                    batch.refresh = true;
                    batch.invalidate |= invalidate;
                }
            }
        }
        batch
    }
    fn needs_discovery(&self) -> bool {
        self.configure || self.refresh || self.invalidate || !self.interfaces.is_empty()
    }
}
fn windows_peripheral(target: &Target) -> Option<&system::Peripheral> {
    match target {
        Target::Windows(p) => Some(p),
        Target::Hidpp(t) => t.windows.as_ref(),
        Target::SteelSeries(t) => t.windows.as_ref(),
        _ => None,
    }
}
fn metadata_wake(target: &Target, containers: &Option<Vec<String>>) -> bool {
    windows_peripheral(target).is_some_and(|p| {
        containers
            .as_ref()
            .is_none_or(|ids| p.container.as_ref().is_some_and(|c| ids.contains(c)))
    })
}
struct ScanClock {
    repair: std::time::Instant,
    topology: Option<std::time::Instant>,
}
impl ScanClock {
    fn new(now: std::time::Instant) -> Self {
        Self {
            repair: now,
            topology: None,
        }
    }
    fn event(&mut self, now: std::time::Instant) {
        self.topology
            .get_or_insert(now + Duration::from_millis(300));
    }
    fn due(&self, now: std::time::Instant) -> bool {
        now >= self.repair || self.topology.is_some_and(|t| now >= t)
    }
    fn wait(&self, now: std::time::Instant) -> Duration {
        self.topology
            .map_or(self.repair, |t| t.min(self.repair))
            .saturating_duration_since(now)
    }
    fn scanned(&mut self, now: std::time::Instant) {
        self.repair = now + Duration::from_secs(300);
        self.topology = None;
    }
}
pub fn spawn(
    rx: Receiver<Command>,
    latest: Latest,
    generation: Arc<AtomicU64>,
    host: (usize, u32),
    controls: Controls,
    commands: SyncSender<Command>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
        struct Com(bool);
        impl Drop for Com {
            fn drop(&mut self) {
                if self.0 {
                    unsafe {
                        CoUninitialize();
                    }
                }
            }
        }
        let _com = Com(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok());
        let dirty = Arc::new(AtomicBool::new(false));
        let changed = dirty.clone();
        let watchers = crate::discovery::start(Arc::new(move || {
            changed.store(true, Ordering::Release);
            let _ = commands.try_send(Command::DiscoveryChanged);
        }))
        .map_err(|e| settings::log_error(&e))
        .ok();
        let gate = Arc::new(Gate::default());
        let publisher = Publisher {
            latest,
            generation,
            host: host.0,
            message: host.1,
        };
        let mut workers: Vec<Worker> = Vec::new();
        let mut clock = ScanClock::new(std::time::Instant::now());
        loop {
            let first = match rx.recv_timeout(clock.wait(std::time::Instant::now())) {
                Ok(command) => Some(command),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => None,
            };
            let batch = Batch::collect(first, &rx);
            let now = std::time::Instant::now();
            if batch.discovery || dirty.swap(false, Ordering::AcqRel) {
                clock.event(now);
            }
            let periodic = now >= clock.repair;
            let needs_discovery = batch.needs_discovery() || clock.due(now);
            if batch.stop {
                break;
            }
            let Batch {
                refresh,
                invalidate,
                interfaces,
                requested,
                ..
            } = batch;
            let expected = publisher.generation.load(Ordering::Acquire);
            if expected == u64::MAX {
                break;
            }
            if !needs_discovery {
                for worker in workers.iter().filter(|w| requested.contains(&w.id)) {
                    worker.refresh();
                }
                continue;
            }
            if invalidate {
                for worker in &workers {
                    worker.cancel();
                }
                workers.clear();
            }
            let metadata = watchers
                .as_ref()
                .map(|w| w.take_metadata())
                .unwrap_or_default();
            let metadata_batteries = metadata.batteries;
            let metadata_changes = metadata.containers.map(|ids| {
                ids.iter()
                    .filter_map(|id| system::container_event_key(id))
                    .collect::<Vec<_>>()
            });
            let targets = discover_with(
                &controls.policy(),
                if periodic { None } else { watchers.as_ref() },
            )
            .or_else(|error| {
                settings::log_error(&error);
                discover_with(&controls.policy(), None)
            });
            match targets {
                Ok(Discovery { targets, catalog }) => {
                    // A reconnect can keep its stable id but replace PnP nodes or HID paths.
                    // Recreate that worker instead of retaining an obsolete device handle.
                    workers.retain(|w| {
                        targets.contains(&w.target) && !w.target.matches_interface(&interfaces)
                    });
                    publisher.change(expected, |state| {
                        state.reconcile(&targets);
                        state.catalog = catalog;
                    });
                    for target in targets {
                        let metadata = windows_peripheral(&target)
                            .and_then(|p| p.container.as_ref())
                            .and_then(|c| metadata_batteries.iter().find(|(id, _)| id == c))
                            .map(|(_, m)| m.clone());
                        if let Some(worker) = workers.iter().find(|w| w.id == target.id()) {
                            if metadata_wake(&target, &metadata_changes)
                                && let Ok(mut current) = worker.metadata.lock()
                            {
                                *current = metadata.clone();
                            }
                            if refresh || requested.contains(&worker.id) {
                                worker.refresh();
                            } else if metadata_wake(&target, &metadata_changes) {
                                let _ = worker.tx.try_send(());
                            }
                        } else {
                            workers.push(Worker::new(
                                target,
                                publisher.clone(),
                                controls.clone(),
                                gate.clone(),
                                metadata,
                            ));
                        }
                    }
                }
                Err(error) => publisher.change(expected, |state| state.error = Some(error)),
            }
            clock.scanned(std::time::Instant::now());
        }
        for worker in &workers {
            worker.cancel();
        }
        workers.clear();
    })
}

pub fn probe_all() -> Result<Vec<Sample>, String> {
    let latest = Arc::new(Mutex::new(None));
    let generation = Arc::new(AtomicU64::new(1));
    let (tx, rx) = mpsc::sync_channel(4);
    let worker = spawn(
        rx,
        latest.clone(),
        generation.clone(),
        (0, 0),
        Controls::new(settings::ProviderPreferences::default()),
        tx.clone(),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let result = loop {
        let snapshot = latest.lock().ok().and_then(|s| s.clone());
        if let Some(snapshot) = snapshot {
            if let Some(error) = snapshot.error {
                break Err(error);
            }
            if snapshot.devices.iter().all(|d| {
                d.result.as_ref().is_ok() || d.result.as_ref().is_err_and(|e| e != "正在查询电量…")
            }) || std::time::Instant::now() >= deadline
            {
                break Ok(snapshot.devices);
            }
        }
        if std::time::Instant::now() >= deadline {
            break Err("设备发现超时".into());
        }
        thread::sleep(Duration::from_millis(20));
    };
    generation.store(u64::MAX, Ordering::Release);
    let _ = tx.send(Command::Stop);
    let _ = worker.join();
    result
}
