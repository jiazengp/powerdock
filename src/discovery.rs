//! Paired Bluetooth AEP and PnP container discovery. Callbacks only patch metadata and wake.
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use windows::{
    Devices::{
        Bluetooth::{BluetoothDevice, BluetoothLEDevice},
        Enumeration::{
            DeviceInformation, DeviceInformationKind, DeviceInformationUpdate, DeviceWatcher,
            DeviceWatcherStatus,
        },
    },
    Foundation::TypedEventHandler,
    core::{HSTRING, IInspectable},
};

type Wake = Arc<dyn Fn() + Send + Sync>;
const MAX_ENDPOINTS: usize = 256;
const STOP_TIMEOUT: Duration = Duration::from_millis(500);
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

// The same keys are requested by the one-shot repair scan in system.rs.
pub const PROPERTIES: [&str; 5] = [
    "System.Devices.Aep.IsConnected",
    "System.Devices.Aep.DeviceAddress",
    "System.Devices.Aep.ContainerId",
    "System.Devices.Aep.Bluetooth.Le.Appearance",
    "System.Devices.Aep.Bluetooth.Le.AddressType",
];

// Public Windows property-system keys; unsupported/missing properties stay absent.
// Container Id is DeviceInformation.Id, not System.Devices.Aep.ContainerId.
// Driver publication of battery changes is best-effort; repair scans remain required.
const CONTAINER_PROPERTIES: [&str; 3] = [
    "System.Devices.BatteryLife",
    "System.Devices.ChargingState",
    "System.Devices.Connected",
];

struct Catalog<T> {
    generation: u64,
    retired: bool,
    entries: BTreeMap<(bool, String), T>,
    changed_ids: BTreeSet<String>,
    all_changed: bool,
    complete: [bool; 2],
    stopped: [bool; 2],
    error: Option<String>,
}
impl<T> Catalog<T> {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            retired: false,
            entries: BTreeMap::new(),
            changed_ids: BTreeSet::new(),
            all_changed: false,
            complete: [false; 2],
            stopped: [false; 2],
            error: None,
        }
    }
    fn mark_changed(&mut self, id: String) {
        if self.all_changed {
            return;
        }
        self.changed_ids.insert(id);
        if self.changed_ids.len() > MAX_ENDPOINTS {
            self.changed_ids.clear();
            self.all_changed = true;
        }
    }
    fn take_changed(&mut self) -> Option<Vec<String>> {
        let ids = std::mem::take(&mut self.changed_ids).into_iter().collect();
        if std::mem::take(&mut self.all_changed) {
            None
        } else {
            Some(ids)
        }
    }
    fn accepts(&self, generation: u64) -> bool {
        !self.retired && self.generation == generation
    }
    fn add(&mut self, generation: u64, le: bool, id: String, item: T) {
        if !self.accepts(generation) {
            return;
        }
        let key = (le, id);
        if self.entries.contains_key(&key) || self.entries.len() < MAX_ENDPOINTS {
            self.mark_changed(key.1.clone());
            self.entries.insert(key, item);
        } else {
            self.error
                .get_or_insert_with(|| "设备发现目录超过 256 项 · truncated".into());
        }
    }
    fn update(
        &mut self,
        generation: u64,
        le: bool,
        id: String,
        patch: impl FnOnce(&T) -> Result<(), String>,
    ) {
        if !self.accepts(generation) {
            return;
        }
        if let Some(result) = self.entries.get(&(le, id.clone())).map(patch) {
            self.mark_changed(id);
            if let Err(error) = result {
                self.error.get_or_insert(error);
            }
        }
    }
    fn remove(&mut self, generation: u64, le: bool, id: String) {
        if self.accepts(generation) && self.entries.remove(&(le, id.clone())).is_some() {
            self.mark_changed(id);
        }
    }
    fn finish(&mut self, generation: u64, le: bool) {
        if self.accepts(generation) {
            self.complete[usize::from(le)] = true;
        }
    }
    fn stopped(&mut self, generation: u64, le: bool) {
        if self.generation == generation {
            self.stopped[usize::from(le)] = true;
            if !self.retired {
                self.error
                    .get_or_insert_with(|| "蓝牙发现已停止 · 使用低频修复扫描".into());
            }
        }
    }
    fn retire(&mut self) {
        self.retired = true;
    }
}

type State = Arc<(Mutex<Catalog<DeviceInformation>>, Condvar)>;

struct Registration {
    watcher: DeviceWatcher,
    le: bool,
    state: State,
    // Partial registration failures also revoke every successfully installed token.
    added: Option<i64>,
    updated: Option<i64>,
    removed: Option<i64>,
    complete: Option<i64>,
    stopped: Option<i64>,
}
impl Drop for Registration {
    fn drop(&mut self) {
        for (kind, token) in [
            (0, self.added),
            (1, self.updated),
            (2, self.removed),
            (3, self.complete),
            (4, self.stopped),
        ] {
            let Some(token) = token else {
                continue;
            };
            let result = match kind {
                0 => self.watcher.RemoveAdded(token),
                1 => self.watcher.RemoveUpdated(token),
                2 => self.watcher.RemoveRemoved(token),
                3 => self.watcher.RemoveEnumerationCompleted(token),
                _ => self.watcher.RemoveStopped(token),
            };
            if let Err(error) = result {
                tracing::warn!(%error, "DeviceWatcher callback removal failed");
            }
        }
    }
}

pub struct Watchers {
    registrations: Vec<Registration>,
    state: State,
    // A separate bounded catalog: container metadata is never returned as a Bluetooth AEP.
    metadata: State,
}

pub struct MetadataChanges {
    pub containers: Option<Vec<String>>,
    pub batteries: Vec<(String, crate::adapters::system::MetadataBattery)>,
}
impl Default for MetadataChanges {
    fn default() -> Self {
        Self {
            containers: Some(Vec::new()),
            batteries: Vec::new(),
        }
    }
}
impl Watchers {
    /// Drain affected identities and parse their merged properties under one lock;
    /// an event cannot be consumed after taking an older property snapshot.
    pub fn take_metadata(&self) -> MetadataChanges {
        let Ok(mut state) = self.metadata.0.lock() else {
            return MetadataChanges {
                containers: None,
                batteries: Vec::new(),
            };
        };
        let batteries = state
            .entries
            .values()
            .filter_map(crate::adapters::system::container_metadata)
            .collect();
        MetadataChanges {
            containers: state.take_changed(),
            batteries,
        }
    }
    /// Partial initial results are usable immediately; overflow/terminal errors require repair.
    /// Parse metadata while holding the same lock used by Updated/Removed callbacks.
    /// The reader must only inspect metadata, and must not call back into Watchers.
    pub fn with_snapshot<R>(
        &self,
        read: impl FnOnce(&[(bool, DeviceInformation)]) -> R,
    ) -> Result<R, String> {
        for registration in &self.registrations {
            match registration.watcher.Status() {
                Ok(DeviceWatcherStatus::Started | DeviceWatcherStatus::EnumerationCompleted) => {}
                Ok(_) => return Err("蓝牙发现已停止 · 使用低频修复扫描".into()),
                Err(error) => return Err(format!("无法读取蓝牙发现状态: {error}")),
            }
        }
        if let Some(error) = &self
            .metadata
            .0
            .lock()
            .map_err(|_| "PnP 发现目录不可用")?
            .error
        {
            return Err(format!("PnP 属性发现: {error}"));
        }
        let state = self.state.0.lock().map_err(|_| "蓝牙发现目录不可用")?;
        if let Some(error) = &state.error {
            return Err(error.clone());
        }
        let entries: Vec<_> = state
            .entries
            .iter()
            .map(|((le, _), info)| (*le, info.clone()))
            .collect();
        let result = read(&entries);
        drop(state);
        Ok(result)
    }

    pub fn initial_complete(&self) -> bool {
        self.state
            .0
            .lock()
            .is_ok_and(|state| state.complete == [true; 2])
    }
}

fn changed(state: &State, wake: &Wake, change: impl FnOnce(&mut Catalog<DeviceInformation>)) {
    if let Ok(mut current) = state.0.lock() {
        change(&mut current);
    }
    // A capacity-one channel coalesces storms; no callback waits for the monitor.
    wake();
}

fn paired_watcher(le: bool) -> windows::core::Result<DeviceWatcher> {
    let selector = if le {
        BluetoothLEDevice::GetDeviceSelectorFromPairingState(true)?
    } else {
        BluetoothDevice::GetDeviceSelectorFromPairingState(true)?
    };
    let requested: windows_collections::IIterable<HSTRING> = PROPERTIES
        .into_iter()
        .map(HSTRING::from)
        .collect::<Vec<_>>()
        .into();
    DeviceInformation::CreateWatcherWithKindAqsFilterAndAdditionalProperties(
        &selector,
        &requested,
        DeviceInformationKind::AssociationEndpoint,
    )
}

fn container_watcher() -> windows::core::Result<DeviceWatcher> {
    // Paired applies to DeviceContainer and excludes the local machine/unpaired PnP nodes.
    // https://learn.microsoft.com/windows/apps/develop/devices-sensors/device-information-properties
    let selector = HSTRING::from("System.Devices.Paired:=System.StructuredQueryType.Boolean#True");
    let requested: windows_collections::IIterable<HSTRING> = CONTAINER_PROPERTIES
        .into_iter()
        .map(HSTRING::from)
        .collect::<Vec<_>>()
        .into();
    DeviceInformation::CreateWatcherWithKindAqsFilterAndAdditionalProperties(
        &selector,
        &requested,
        DeviceInformationKind::DeviceContainer,
    )
}

fn register(
    watcher: DeviceWatcher,
    le: bool,
    generation: u64,
    state: &State,
    wake: &Wake,
) -> windows::core::Result<Registration> {
    let mut registration = Registration {
        watcher,
        le,
        state: state.clone(),
        added: None,
        updated: None,
        removed: None,
        complete: None,
        stopped: None,
    };
    let (shared, signal) = (state.clone(), wake.clone());
    registration.added = Some(registration.watcher.Added(&TypedEventHandler::<
        DeviceWatcher,
        DeviceInformation,
    >::new(move |_, info| {
        if let Some(info) = info.as_ref()
            && let Ok(id) = info.Id()
        {
            changed(&shared, &signal, |state| {
                state.add(generation, le, id.to_string(), info.clone())
            });
        }
        Ok(())
    }))?);
    let (shared, signal) = (state.clone(), wake.clone());
    registration.updated = Some(registration.watcher.Updated(&TypedEventHandler::<
        DeviceWatcher,
        DeviceInformationUpdate,
    >::new(move |_, update| {
        if let Some(update) = update.as_ref()
            && let Ok(id) = update.Id()
        {
            changed(&shared, &signal, |state| {
                state.update(generation, le, id.to_string(), |info| {
                    info.Update(update)
                        .map_err(|error| format!("设备发现属性更新失败: {error}"))
                })
            });
        }
        Ok(())
    }))?);
    let (shared, signal) = (state.clone(), wake.clone());
    registration.removed = Some(registration.watcher.Removed(&TypedEventHandler::<
        DeviceWatcher,
        DeviceInformationUpdate,
    >::new(move |_, update| {
        if let Some(update) = update.as_ref()
            && let Ok(id) = update.Id()
        {
            changed(&shared, &signal, |state| {
                state.remove(generation, le, id.to_string())
            });
        }
        Ok(())
    }))?);
    let (shared, signal) = (state.clone(), wake.clone());
    registration.complete = Some(registration.watcher.EnumerationCompleted(
        &TypedEventHandler::<DeviceWatcher, IInspectable>::new(move |_, _| {
            changed(&shared, &signal, |state| state.finish(generation, le));
            Ok(())
        }),
    )?);
    let (shared, signal) = (state.clone(), wake.clone());
    registration.stopped = Some(registration.watcher.Stopped(&TypedEventHandler::<
        DeviceWatcher,
        IInspectable,
    >::new(move |_, _| {
        changed(&shared, &signal, |state| state.stopped(generation, le));
        shared.1.notify_all();
        Ok(())
    }))?);
    Ok(registration)
}

/// The caller owns its COM apartment and drops these watchers before uninitializing it.
pub fn start(wake: Wake) -> Result<Watchers, String> {
    let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
    let mut watchers = Watchers {
        registrations: Vec::new(),
        state: Arc::new((Mutex::new(Catalog::new(generation)), Condvar::new())),
        metadata: Arc::new((Mutex::new(Catalog::new(generation)), Condvar::new())),
    };
    // Install all callbacks on both watchers before either starts publishing results.
    for le in [false, true] {
        watchers.registrations.push(
            paired_watcher(le)
                .and_then(|watcher| register(watcher, le, generation, &watchers.state, &wake))
                .map_err(|error| format!("无法注册蓝牙发现: {error}"))?,
        );
    }
    watchers.registrations.push(
        container_watcher()
            .and_then(|watcher| register(watcher, false, generation, &watchers.metadata, &wake))
            .map_err(|error| format!("无法注册 PnP 属性发现: {error}"))?,
    );
    for registration in &watchers.registrations {
        registration
            .watcher
            .Start()
            .map_err(|error| format!("无法启动蓝牙发现: {error}"))?;
    }
    Ok(watchers)
}

impl Drop for Watchers {
    fn drop(&mut self) {
        // Locking serializes retirement with in-flight patches; no later callback revives an endpoint.
        if let Ok(mut state) = self.state.0.lock() {
            state.retire();
        }
        if let Ok(mut state) = self.metadata.0.lock() {
            state.retire();
        }
        for registration in &self.registrations {
            match registration.watcher.Status() {
                Ok(DeviceWatcherStatus::Started | DeviceWatcherStatus::EnumerationCompleted) => {
                    if let Err(error) = registration.watcher.Stop() {
                        tracing::warn!(%error, "DeviceWatcher Stop failed");
                    }
                }
                Ok(DeviceWatcherStatus::Stopping) => {}
                Ok(
                    DeviceWatcherStatus::Created
                    | DeviceWatcherStatus::Stopped
                    | DeviceWatcherStatus::Aborted,
                ) => {
                    if let Ok(mut state) = registration.state.0.lock() {
                        state.stopped[usize::from(registration.le)] = true;
                    }
                }
                status => {
                    tracing::warn!(?status, "DeviceWatcher status unavailable during shutdown");
                    if let Err(error) = registration.watcher.Stop() {
                        tracing::warn!(%error, "DeviceWatcher Stop failed");
                    }
                }
            }
        }
        let deadline = Instant::now() + STOP_TIMEOUT;
        for registration in &self.registrations {
            let Ok(mut state) = registration.state.0.lock() else {
                continue;
            };
            while !state.stopped[usize::from(registration.le)] {
                let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                    break;
                };
                let Ok((current, _)) = registration.state.1.wait_timeout(state, remaining) else {
                    return;
                };
                state = current;
            }
            if !state.stopped[usize::from(registration.le)] {
                tracing::warn!(
                    "DeviceWatcher Stopped deadline expired; revoking callbacks, never restarting the stopping watcher"
                );
            }
        }
        // Registration::drop unregisters after waiting. No application callback thread is spawned.
    }
}

#[cfg(test)]
mod tests;
