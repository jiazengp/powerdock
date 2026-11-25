//! Lazy receiver registry and generation-fenced transactions.
use super::features::{CachedDevice, channel_error};
use super::{Device, Observation, RECEIVER_PAUSED, eligible_collection};
use hidpp::channel::HidppChannel;
use std::{
    collections::BTreeMap,
    future::Future,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const TRANSACTION_BUDGET: Duration = Duration::from_secs(3);
const REGISTRY_LIMIT: usize = 16;

struct SharedReceiver {
    state: Mutex<State>,
}
#[derive(Default)]
struct State {
    channel: Option<Arc<HidppChannel>>,
    devices: BTreeMap<u8, CachedDevice>,
    lifetime: Arc<AtomicU64>,
}
impl State {
    fn retire(&mut self) {
        self.lifetime.fetch_add(1, Ordering::AcqRel);
        self.devices.clear();
        self.channel = None;
    }
}
static RECEIVERS: OnceLock<Mutex<BTreeMap<String, Weak<SharedReceiver>>>> = OnceLock::new();
pub struct Session {
    device: Device,
    shared: Arc<SharedReceiver>,
}
impl Session {
    /// Lazy metadata/session acquisition. No hardware is queried here.
    pub fn open(device: &Device) -> Result<Self, String> {
        if !device.monitor_eligible() {
            return Err(RECEIVER_PAUSED.into());
        }
        if device.endpoints.is_empty() || device.endpoints.iter().any(|d| !eligible_collection(d)) {
            return Err("unsupported: HID++ collection".into());
        }
        let mut registry = RECEIVERS
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
            .map_err(|_| "busy: HID++ registry")?;
        registry.retain(|_, weak| weak.strong_count() > 0);
        let shared = if let Some(shared) = registry.get(&device.group).and_then(Weak::upgrade) {
            shared
        } else {
            if registry.len() >= REGISTRY_LIMIT {
                return Err("budget_deferred: HID++ receiver registry".into());
            }
            let shared = Arc::new(SharedReceiver {
                state: Mutex::new(State::default()),
            });
            registry.insert(device.group.clone(), Arc::downgrade(&shared));
            shared
        };
        Ok(Self {
            device: device.clone(),
            shared,
        })
    }
    pub fn query(&self, generation: &AtomicU64, expected: u64) -> Result<Observation, String> {
        let deadline = Instant::now() + TRANSACTION_BUDGET;
        let mut state = loop {
            check(generation, expected, deadline)?;
            match self.shared.state.try_lock() {
                Ok(state) => break state,
                Err(std::sync::TryLockError::WouldBlock) => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(_) => return Err("busy: HID++ receiver session".into()),
            }
        };
        if state.channel.is_none() {
            let lease = state.lifetime.load(Ordering::Acquire);
            let raw =
                super::transport::open(&self.device.endpoints, state.lifetime.clone(), lease)?;
            check(generation, expected, deadline)?;
            state.channel = Some(Arc::new(drive(
                async {
                    HidppChannel::from_raw_channel(raw)
                        .await
                        .map_err(channel_error)
                },
                generation,
                expected,
                deadline,
            )?));
        }
        let result = (|| {
            let channel = state
                .channel
                .as_ref()
                .ok_or("disconnected: HID++ channel")?
                .clone();
            if let std::collections::btree_map::Entry::Vacant(entry) =
                state.devices.entry(self.device.slot)
            {
                let cached = drive(
                    CachedDevice::initialize(channel, self.device.slot),
                    generation,
                    expected,
                    deadline,
                )?;
                entry.insert(cached);
            }
            let cached = state
                .devices
                .get(&self.device.slot)
                .ok_or("disconnected: HID++ slot")?;
            drive(cached.read(&self.device), generation, expected, deadline)
        })();
        if result.is_err() {
            state.retire();
        }
        result
    }
}
fn check(generation: &AtomicU64, expected: u64, deadline: Instant) -> Result<(), String> {
    if generation.load(Ordering::Acquire) != expected {
        Err("cancelled: HID++ transaction generation".into())
    } else if Instant::now() >= deadline {
        Err("timeout: HID++ transaction deadline".into())
    } else {
        Ok(())
    }
}
pub(super) fn drive<T, E: Into<String>>(
    future: impl Future<Output = Result<T, E>>,
    generation: &AtomicU64,
    expected: u64,
    deadline: Instant,
) -> Result<T, String> {
    futures::executor::block_on(async {
        let mut future = Box::pin(future);
        let request = Box::pin(futures::future::poll_fn(|cx| {
            if let Err(error) = check(generation, expected, deadline) {
                return std::task::Poll::Ready(Err(error));
            }
            match future.as_mut().poll(cx) {
                std::task::Poll::Ready(result) => {
                    std::task::Poll::Ready(result.map_err(Into::into))
                }
                std::task::Poll::Pending => std::task::Poll::Pending,
            }
        }));
        let cancelled = Box::pin(async {
            loop {
                check(generation, expected, deadline)?;
                futures_timer::Delay::new(Duration::from_millis(10)).await;
            }
        });
        match futures::future::select(request, cancelled).await {
            futures::future::Either::Left((result, _)) => {
                check(generation, expected, deadline)?;
                result
            }
            futures::future::Either::Right((result, _)) => {
                let result: Result<(), String> = result;
                Err(result
                    .err()
                    .unwrap_or_else(|| "cancelled: HID++ transaction".into()))
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::super::{enumerate_from, tests::interface};
    use super::*;
    #[test]
    fn shared_registry_reuses_receiver_gate_without_holding_hardware_resources() {
        let device = enumerate_from(&[interface("registry-direct", 0xb001, 1)])[0].clone();
        let a = Session::open(&device).unwrap();
        let b = Session::open(&device).unwrap();
        assert!(Arc::ptr_eq(&a.shared, &b.shared));
        assert!(a.shared.state.lock().unwrap().channel.is_none());
        assert!(!format!("{device:?}").contains("synthetic"));
    }
}
