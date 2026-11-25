//! Windows characteristic reads, notification cache and subscription teardown.
use super::*;
use std::sync::Mutex;
use windows::{
    Devices::Bluetooth::GenericAttributeProfile::{
        GattCharacteristic, GattCharacteristicProperties,
        GattClientCharacteristicConfigurationDescriptorValue, GattValueChangedEventArgs,
    },
    Storage::Streams::{DataReader, IBuffer},
};

pub(super) fn bytes(buffer: &IBuffer) -> Result<Vec<u8>, String> {
    let length = buffer.Length().map_err(|_| "invalid_reply: GATT length")?;
    if length == 0 || length > 32 {
        return Err("invalid_reply: GATT payload size".into());
    }
    let reader = DataReader::FromBuffer(buffer).map_err(|_| "invalid_reply: GATT buffer")?;
    let mut data = vec![0; length as usize];
    let result = reader.ReadBytes(&mut data);
    let _ = reader.Close();
    result.map_err(|_| "invalid_reply: GATT bytes")?;
    Ok(data)
}
pub(super) fn accept_notification(
    cache: &Mutex<Option<Packet>>,
    epoch: &AtomicU64,
    version: u64,
    data: Vec<u8>,
) -> bool {
    let Ok(mut slot) = cache.lock() else {
        return false;
    };
    if epoch.load(Ordering::Acquire) != version {
        return false;
    }
    *slot = Some(Packet {
        bytes: data,
        at: crate::settings::now(),
        clock: Instant::now(),
    });
    true
}
pub(super) struct Characteristic {
    pub(super) value: GattCharacteristic,
    pub(super) latest: Arc<Mutex<Option<Packet>>>,
    pub(super) token: Option<i64>,
    pub(super) attempted: bool,
}
impl Drop for Characteristic {
    fn drop(&mut self) {
        if let Some(token) = self.token.take() {
            let _ = self.value.RemoveValueChanged(token);
        }
    }
}
impl Characteristic {
    pub(super) fn new(value: GattCharacteristic) -> Result<Self, String> {
        if !value
            .CharacteristicProperties()
            .is_ok_and(|p| p.0 & GattCharacteristicProperties::Read.0 != 0)
        {
            return Err("unsupported: BAS characteristic is not readable".into());
        }
        Ok(Self {
            value,
            latest: Arc::new(Mutex::new(None)),
            token: None,
            attempted: false,
        })
    }
    pub(super) fn read(
        &self,
        uncached: bool,
        deadline: Instant,
        generation: &AtomicU64,
        expected: u64,
    ) -> Result<Packet, String> {
        if !uncached
            && let Some(packet) = self.latest.lock().ok().and_then(|p| p.clone())
            && packet.clock.elapsed() < crate::battery::FRESH_FOR
        {
            return Ok(packet);
        }
        let result = crate::winrt::wait(
            self.value
                .ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached),
            deadline,
            generation,
            expected,
        )?;
        if result.Status().ok() != Some(GattCommunicationStatus::Success) {
            return Err("invalid_reply: BAS read failed".into());
        }
        let packet = Packet {
            bytes: bytes(&result.Value().map_err(|_| "invalid_reply: BAS buffer")?)?,
            at: crate::settings::now(),
            clock: Instant::now(),
        };
        *self.latest.lock().map_err(|_| "BAS cache unavailable")? = Some(packet.clone());
        Ok(packet)
    }
    pub(super) fn subscribe(
        &mut self,
        epoch: &Arc<AtomicU64>,
        wake: &Option<Wake>,
        deadline: Instant,
        generation: &AtomicU64,
        expected: u64,
    ) {
        if self.attempted {
            return;
        }
        self.attempted = true;
        let Ok(props) = self.value.CharacteristicProperties() else {
            return;
        };
        let mode = if props.0 & GattCharacteristicProperties::Notify.0 != 0 {
            GattClientCharacteristicConfigurationDescriptorValue::Notify
        } else if props.0 & GattCharacteristicProperties::Indicate.0 != 0 {
            GattClientCharacteristicConfigurationDescriptorValue::Indicate
        } else {
            return;
        };
        let (active, version, cache, wake) = (
            epoch.clone(),
            epoch.load(Ordering::Acquire),
            self.latest.clone(),
            wake.clone(),
        );
        let Ok(token) = self.value.ValueChanged(&TypedEventHandler::<
            GattCharacteristic,
            GattValueChangedEventArgs,
        >::new(move |_, args| {
            if active.load(Ordering::Acquire) != version {
                return Ok(());
            }
            if let Some(args) = args.as_ref()
                && let Ok(buffer) = args.CharacteristicValue()
                && let Ok(data) = bytes(&buffer)
                && accept_notification(&cache, &active, version, data)
                && let Some(wake) = &wake
            {
                wake();
            }
            Ok(())
        })) else {
            return;
        };
        let success = crate::winrt::wait(
            self.value
                .WriteClientCharacteristicConfigurationDescriptorAsync(mode),
            deadline,
            generation,
            expected,
        ) == Ok(GattCommunicationStatus::Success);
        if success {
            self.token = Some(token);
        } else {
            let _ = self.value.RemoveValueChanged(token);
        }
        crate::diagnostics::record(
            "gatt_subscription",
            serde_json::json!({"success":success,"mode":mode.0}),
        );
    }
}
pub(super) struct Service(pub(super) GattDeviceService);
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.Close();
    }
}
