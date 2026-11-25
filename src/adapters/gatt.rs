//! Standard BAS reads and bounded CCCD subscriptions; never maintain a forced connection.
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use windows::Devices::Bluetooth::{
    BluetoothCacheMode, BluetoothLEDevice,
    GenericAttributeProfile::{GattCommunicationStatus, GattDeviceService, GattServiceUuids},
};

mod bas;
mod characteristic;
use crate::battery::{Component, Kind, Reading};
use bas::{aggregate, component_value, cpf, level, status};
use characteristic::{Characteristic, Service, bytes};
use std::sync::{Arc, atomic::AtomicBool};
use windows::{
    Foundation::TypedEventHandler,
    core::{GUID, IInspectable},
};
pub type Wake = Arc<dyn Fn() + Send + Sync>;
fn uuid(short: u16) -> GUID {
    GUID::from_u128(((short as u128) << 96) | 0x00000000_0000_1000_8000_00805f9b34fb)
}
#[derive(Clone)]
struct Packet {
    bytes: Vec<u8>,
    at: u64,
    clock: Instant,
}
struct Battery {
    level: Option<Characteristic>,
    status: Option<Characteristic>,
    description: Option<u16>,
    handle: u16,
    _service: Service,
}
pub struct Frame {
    pub reading: Reading,
    pub components: Vec<Component>,
    pub at: u64,
    pub clock: Instant,
    pub subscribed: bool,
}
fn requires_read(subscribed: bool, last: Option<Instant>, now: Instant) -> bool {
    !subscribed
        || last.is_none_or(|at| now.saturating_duration_since(at) >= crate::battery::FRESH_FOR)
}
pub struct Session {
    batteries: Vec<Battery>,
    epoch: Arc<AtomicU64>,
    invalid: Arc<AtomicBool>,
    service_token: Option<(BluetoothLEDevice, i64)>,
    last_read: Option<Instant>,
    wake: Option<Wake>,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            batteries: vec![],
            epoch: Arc::new(AtomicU64::new(1)),
            invalid: Arc::new(AtomicBool::new(false)),
            service_token: None,
            last_read: None,
            wake: None,
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.clear();
    }
}
impl Session {
    pub fn require_uncached_read(&mut self) {
        self.last_read = None;
    }
    fn clear(&mut self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.batteries.clear();
        self.last_read = None;
        if let Some((device, token)) = self.service_token.take() {
            let _ = device.RemoveGattServicesChanged(token);
        }
    }
    pub fn read(
        &mut self,
        device: &BluetoothLEDevice,
        kind: Kind,
        extended: bool,
        wake: Option<Wake>,
        generation: &AtomicU64,
        expected: u64,
    ) -> Result<Frame, String> {
        self.wake = wake;
        if self.invalid.swap(false, Ordering::AcqRel) {
            self.clear();
        }
        let result = self.read_inner(device, kind, extended, generation, expected);
        if result.is_err() {
            self.clear();
        }
        result
    }
    fn read_inner(
        &mut self,
        device: &BluetoothLEDevice,
        kind: Kind,
        extended: bool,
        generation: &AtomicU64,
        expected: u64,
    ) -> Result<Frame, String> {
        let deadline = Instant::now() + Duration::from_secs(3);
        if self.batteries.is_empty() {
            let (invalid, wake) = (self.invalid.clone(), self.wake.clone());
            if self.service_token.is_none()
                && let Ok(token) =
                    device.GattServicesChanged(
                        &TypedEventHandler::<BluetoothLEDevice, IInspectable>::new(move |_, _| {
                            invalid.store(true, Ordering::Release);
                            if let Some(w) = &wake {
                                w();
                            }
                            Ok(())
                        }),
                    )
            {
                self.service_token = Some((device.clone(), token));
            }
            let services = crate::winrt::wait(
                device.GetGattServicesForUuidWithCacheModeAsync(
                    GattServiceUuids::Battery().map_err(|_| "unsupported: BAS UUID")?,
                    BluetoothCacheMode::Cached,
                ),
                deadline,
                generation,
                expected,
            )?;
            if services.Status().ok() != Some(GattCommunicationStatus::Success) {
                return Err("unsupported: BAS unavailable".into());
            }
            let list = services
                .Services()
                .map_err(|_| "unsupported: BAS service list")?;
            let mut pending: Vec<_> = list.into_iter().map(Service).collect();
            if pending.is_empty() || pending.len() > 4 {
                return Err("unsupported: BAS count outside 1..4".into());
            }
            while let Some(service) = pending.pop() {
                let chars = crate::winrt::wait(
                    service
                        .0
                        .GetCharacteristicsWithCacheModeAsync(BluetoothCacheMode::Cached),
                    deadline,
                    generation,
                    expected,
                )?;
                if chars.Status().ok() != Some(GattCommunicationStatus::Success) {
                    return Err("unsupported: BAS characteristics".into());
                }
                let chars = chars
                    .Characteristics()
                    .map_err(|_| "unsupported: BAS characteristics list")?;
                if chars.Size().unwrap_or(33) > 32 {
                    return Err("unsupported: BAS characteristic count".into());
                }
                let (mut battery_level, mut battery_status) = (None, None);
                for ch in chars {
                    let id = ch
                        .Uuid()
                        .map_err(|_| "invalid_reply: BAS characteristic UUID")?;
                    let slot = if id == uuid(0x2a19) {
                        Some(&mut battery_level)
                    } else if id == uuid(0x2bed) && extended {
                        Some(&mut battery_status)
                    } else {
                        None
                    };
                    if let Some(slot) = slot {
                        if slot.is_some() {
                            return Err("invalid_reply: duplicate BAS characteristic".into());
                        }
                        *slot = Some(Characteristic::new(ch)?);
                    }
                }
                if battery_level.is_none() && battery_status.is_none() {
                    return Err("unsupported: no readable battery characteristic".into());
                }
                let mut description = None;
                if let Some(level) = &battery_level {
                    let desc = crate::winrt::wait(
                        level.value.GetDescriptorsForUuidWithCacheModeAsync(
                            uuid(0x2904),
                            BluetoothCacheMode::Cached,
                        ),
                        deadline,
                        generation,
                        expected,
                    )?;
                    if desc.Status().ok() != Some(GattCommunicationStatus::Success) {
                        return Err("unsupported: BAS presentation unavailable".into());
                    }
                    let desc = desc
                        .Descriptors()
                        .map_err(|_| "invalid_reply: BAS descriptors")?;
                    if desc.Size().unwrap_or(2) > 1 {
                        return Err("invalid_reply: duplicate BAS presentation".into());
                    }
                    if desc.Size().ok() == Some(1) {
                        let read = crate::winrt::wait(
                            desc.GetAt(0)
                                .map_err(|_| "invalid_reply: BAS descriptor")?
                                .ReadValueWithCacheModeAsync(BluetoothCacheMode::Cached),
                            deadline,
                            generation,
                            expected,
                        )?;
                        if read.Status().ok() != Some(GattCommunicationStatus::Success) {
                            return Err("unsupported: BAS presentation read".into());
                        }
                        description = Some(cpf(&bytes(
                            &read
                                .Value()
                                .map_err(|_| "invalid_reply: BAS presentation buffer")?,
                        )?)?);
                    }
                }
                let handle = service
                    .0
                    .AttributeHandle()
                    .map_err(|_| "invalid_reply: BAS instance")?;
                self.batteries.push(Battery {
                    level: battery_level,
                    status: battery_status,
                    description,
                    handle,
                    _service: service,
                });
            }
        }
        let subscribed = self.batteries.iter().all(|b| {
            b.level
                .iter()
                .chain(b.status.iter())
                .all(|c| c.token.is_some())
        });
        let uncached = requires_read(subscribed, self.last_read, Instant::now());
        let mut components = Vec::new();
        let mut oldest = None;
        for battery in &mut self.batteries {
            let lp = battery
                .level
                .as_ref()
                .map(|c| c.read(uncached, deadline, generation, expected))
                .transpose()?;
            let sp = battery
                .status
                .as_ref()
                .map(|c| c.read(uncached, deadline, generation, expected))
                .transpose()?;
            for p in lp.iter().chain(sp.iter()) {
                if oldest.is_none_or(|(_, clock)| p.clock < clock) {
                    oldest = Some((p.at, p.clock));
                }
            }
            let level = lp.as_ref().map(|p| level(&p.bytes)).transpose()?;
            let status = sp.as_ref().map(|p| status(&p.bytes)).transpose()?;
            let (value, power, external, identifier) =
                component_value(level, status, battery.description)?;
            let id = identifier.map_or_else(
                || format!("bas-instance-{}", battery.handle),
                |id| format!("bas-{id:04x}"),
            );
            let role = if extended {
                match identifier {
                    Some(0x0106) => "main",
                    Some(0x010d) => "left",
                    Some(0x010e) => "right",
                    _ => "unknown",
                }
            } else {
                "unknown"
            };
            components.push(Component {
                id,
                role,
                value,
                power,
                external_power: external,
                identifier,
                raw_power_state: status.map(|s| s.raw),
            });
        }
        components.sort_by(|a, b| a.id.cmp(&b.id));
        let reading = aggregate(&components, kind)?;
        if generation.load(Ordering::Acquire) != expected || Instant::now() >= deadline {
            return Err("cancelled: BAS late read".into());
        }
        if uncached {
            self.last_read = Some(Instant::now());
        }
        for battery in &mut self.batteries {
            for c in battery.level.iter_mut().chain(battery.status.iter_mut()) {
                c.subscribe(&self.epoch, &self.wake, deadline, generation, expected);
            }
        }
        if generation.load(Ordering::Acquire) != expected || self.invalid.load(Ordering::Acquire) {
            return Err("cancelled: BAS late completion".into());
        }
        let (at, clock) = oldest.ok_or("invalid_reply: empty BAS")?;
        Ok(Frame {
            reading,
            components,
            at,
            clock,
            subscribed: self.batteries.iter().all(|b| {
                b.level
                    .iter()
                    .chain(b.status.iter())
                    .all(|c| c.token.is_some())
            }),
        })
    }
}
#[cfg(test)]
mod tests;
