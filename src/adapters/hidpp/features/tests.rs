use super::super::{enumerate_from, session::drive, tests::interface};
use super::*;
use hidpp::{async_trait, channel::RawHidChannel, feature::unified_battery::BatteryEvent};
use std::{
    error::Error,
    sync::{
        Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

#[test]
fn unit_identity_requires_version_and_non_sentinel_model_and_unit() {
    assert!(unit_identity(0, [1, 2, 3, 4], [1, 2, 3]).is_none());
    assert!(unit_identity(1, [0; 4], [1, 2, 3]).is_none());
    assert!(unit_identity(1, [255; 4], [1, 2, 3]).is_none());
    assert!(unit_identity(1, [1, 2, 3, 4], [0; 3]).is_none());
    let Evidence::Unit(a) = unit_identity(1, [1, 2, 3, 4], [0xb001, 0xb002, 0]).unwrap() else {
        panic!("strong")
    };
    let Evidence::Unit(b) = unit_identity(1, [1, 2, 3, 4], [0xb002, 0, 0xb001]).unwrap() else {
        panic!("strong")
    };
    assert_eq!(a, b);
    let Evidence::Unit(other) = unit_identity(1, [1, 2, 3, 5], [0xb001, 0xb002, 0]).unwrap() else {
        panic!("strong")
    };
    assert_ne!(a, other);
}
fn unified(raw: u8, level: u8, status: u8) -> BatteryInfo {
    let mut payload = [0; 16];
    payload[..3].copy_from_slice(&[raw, level, status]);
    match BatteryEvent::decode(0, &payload).expect("fixture public decoder") {
        BatteryEvent::InfoUpdate(info) => info,
        _ => panic!("unknown event"),
    }
}
#[test]
fn unified_capability_raw_range_coarse_and_faults_never_fabricate_percentages() {
    let gauge = BatteryCapabilities::from([15, 3]);
    for raw in [0, 73, 100] {
        assert_eq!(
            map_unified(&gauge, unified(raw, 4, 0), Kind::Mouse).percent(),
            Some(raw)
        );
    }
    assert_eq!(
        map_unified(&gauge, unified(255, 4, 0), Kind::Mouse).value,
        Value::Unknown
    );
    let coarse = BatteryCapabilities::from([15, 1]);
    for (level, expected) in [
        (1, Level::Critical),
        (2, Level::Low),
        (4, Level::High),
        (8, Level::Full),
    ] {
        let r = map_unified(&coarse, unified(0, level, 0), Kind::Mouse);
        assert_eq!(r.percent(), None);
        assert_eq!(r.value, Value::Ordinal { level: expected });
    }
    let unadvertised = BatteryCapabilities::from([1, 0]);
    assert_eq!(
        map_unified(&unadvertised, unified(0, 4, 0), Kind::Mouse).value,
        Value::Unknown
    );
    for status in [5, 6, 7] {
        let r = map_unified(&gauge, unified(10, 4, status), Kind::Mouse);
        assert_eq!(r.value, Value::Unknown);
        assert_eq!(r.power, Power::Unknown);
    }
    for status in [1, 2, 4] {
        assert_eq!(
            map_unified(&gauge, unified(89, 4, status), Kind::Mouse).power,
            Power::Charging
        );
    }
    let full = map_unified(&gauge, unified(97, 8, 3), Kind::Mouse);
    assert_eq!(full.percent(), Some(97));
    assert_eq!(full.power, Power::Full);
}
struct Scripted {
    incoming: async_channel::Receiver<Vec<u8>>,
    reply: async_channel::Sender<Vec<u8>>,
    writes: Arc<Mutex<Vec<Vec<u8>>>>,
    legacy: bool,
    wire: [u8; 3],
    caps: [u8; 2],
}
#[async_trait]
impl RawHidChannel for Scripted {
    fn vendor_id(&self) -> u16 {
        0x046d
    }
    fn product_id(&self) -> u16 {
        0xb001
    }
    fn supports_short_long_hidpp(&self) -> Option<(bool, bool)> {
        Some((true, true))
    }
    async fn write_report(&self, src: &[u8]) -> Result<usize, Box<dyn Error + Sync + Send>> {
        self.writes.lock().unwrap().push(src.to_vec());
        let mut out = vec![0; 20];
        out[0] = 0x11;
        out[1..4].copy_from_slice(&src[1..4]);
        match (src[2], src[3] >> 4) {
            (0, 1) => out[4] = 2,
            (0, 0) => {
                let id = u16::from_be_bytes([src[4], src[5]]);
                out[4] = match id {
                    0x0003 => 3,
                    0x0005 => 5,
                    0x1004 if !self.legacy => 7,
                    0x1000 if self.legacy => 7,
                    _ => 0,
                };
                out[6] = u8::from(id == 0x0003);
            }
            (3, 0) => {
                out[4] = 1;
                out[5..9].copy_from_slice(&[1, 2, 3, 4]);
                out[11..13].copy_from_slice(&[0xb0, 1]);
            }
            (5, 2) => out[4] = 3,
            (7, 0) if !self.legacy => out[4..6].copy_from_slice(&self.caps),
            (7, 1) if !self.legacy => out[4..7].copy_from_slice(&self.wire),
            (7, 1) if self.legacy => out[4..6].copy_from_slice(&self.caps),
            (7, 0) if self.legacy => out[4..7].copy_from_slice(&self.wire),
            _ => panic!("unexpected non-GET request"),
        }
        self.reply.try_send(out).unwrap();
        Ok(src.len())
    }
    async fn read_report(&self, buf: &mut [u8]) -> Result<usize, Box<dyn Error + Sync + Send>> {
        match self.incoming.recv().await {
            Ok(report) => {
                buf[..report.len()].copy_from_slice(&report);
                Ok(report.len())
            }
            Err(_) => futures::future::pending().await,
        }
    }
    async fn get_report_descriptor(
        &self,
        _: &mut [u8],
    ) -> Result<usize, Box<dyn Error + Sync + Send>> {
        panic!("known descriptor capabilities")
    }
}
type Writes = Arc<Mutex<Vec<Vec<u8>>>>;
fn channel(legacy: bool, wire: [u8; 3], caps: [u8; 2]) -> (Arc<HidppChannel>, Writes) {
    let (reply, incoming) = async_channel::bounded(32);
    let writes = Arc::new(Mutex::new(Vec::new()));
    let raw = Scripted {
        incoming,
        reply,
        writes: writes.clone(),
        legacy,
        wire,
        caps,
    };
    (
        Arc::new(futures::executor::block_on(HidppChannel::from_raw_channel(raw)).unwrap()),
        writes,
    )
}
#[test]
fn real_public_protocol_flow_uses_only_root_identity_and_battery_gets() {
    let (channel, writes) = channel(false, [67, 4, 0], [15, 3]);
    let generation = AtomicU64::new(1);
    let deadline = Instant::now() + Duration::from_secs(2);
    let cached = drive(
        CachedDevice::initialize(channel, 0xff),
        &generation,
        1,
        deadline,
    )
    .unwrap();
    let device = enumerate_from(&[interface("direct", 0xb001, 1)])[0].clone();
    let observation = drive(cached.read(&device), &generation, 1, deadline).unwrap();
    assert_eq!(observation.reading.percent(), Some(67));
    assert_eq!(observation.reading.kind, Kind::Mouse);
    assert!(matches!(observation.identity, Evidence::Unit(_)));
    let requests = writes.lock().unwrap();
    assert!(!requests.is_empty());
    assert!(
        requests.iter().all(|r| r[1] == 0xff
            && matches!((r[2], r[3] >> 4), (0, 0 | 1) | (3, 0) | (5, 2) | (7, 0 | 1)))
    );
}
#[test]
fn legacy_capability_and_charging_zero_are_not_false_percentages() {
    for (caps, wire, expected) in [
        ([4, 4], [25, 5, 0], Value::Ordinal { level: Level::Low }),
        ([4, 2], [30, 20, 0], Value::Ordinal { level: Level::High }),
        ([100, 2], [73, 72, 0], Value::reported(73)),
        ([100, 2], [255, 0, 0], Value::Unknown),
        ([100, 2], [0, 0, 1], Value::Unknown),
        ([100, 2], [0, 0, 3], Value::Unknown),
    ] {
        let (channel, _) = channel(true, wire, caps);
        let generation = AtomicU64::new(1);
        let deadline = Instant::now() + Duration::from_secs(2);
        let cached = drive(
            CachedDevice::initialize(channel, 0xff),
            &generation,
            1,
            deadline,
        )
        .unwrap();
        let device = enumerate_from(&[interface("direct", 0xb001, 1)])[0].clone();
        let observation = drive(cached.read(&device), &generation, 1, deadline).unwrap();
        assert_eq!(observation.reading.value, expected);
    }
}
#[test]
fn legacy_register_decoder_keeps_coarse_and_unknown_values_distinct() {
    use hidpp::register::battery::{ChargeReading, ChargingState, StatusLevel, StatusReading};
    let coarse = StatusReading::try_from([3, 0, 0]).unwrap();
    assert_eq!(coarse.level, Some(StatusLevel::Low));
    assert_eq!(coarse.charging, ChargingState::Discharging);
    let charging = StatusReading::try_from([0, 0x21, 0]).unwrap();
    assert_eq!(charging.level, None);
    assert_eq!(charging.charging, ChargingState::Recharging);
    for raw in [0, 75, 100] {
        assert_eq!(
            ChargeReading::try_from([raw, 0, 0x30]).unwrap().percentage,
            raw
        );
    }
    assert!(ChargeReading::try_from([255, 0, 0x30]).is_err());
    assert!(StatusReading::try_from([0, 0x01, 0]).is_err());
}
#[test]
fn transaction_deadline_and_precancel_do_not_poll_or_publish_late_work() {
    let generation = AtomicU64::new(2);
    let polls = AtomicUsize::new(0);
    let result = drive(
        async {
            polls.fetch_add(1, Ordering::SeqCst);
            Ok::<_, String>(())
        },
        &generation,
        1,
        Instant::now() + Duration::from_secs(1),
    );
    assert!(result.unwrap_err().starts_with("cancelled:"));
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    let start = Instant::now();
    let result = drive(
        futures::future::pending::<Result<(), String>>(),
        &generation,
        2,
        start + Duration::from_millis(25),
    );
    assert!(result.unwrap_err().starts_with("timeout:"));
    assert!(start.elapsed() < Duration::from_secs(1));
    let result = drive(
        async {
            futures_timer::Delay::new(Duration::from_millis(40)).await;
            Ok::<_, String>(17)
        },
        &generation,
        2,
        Instant::now() + Duration::from_millis(15),
    );
    assert!(result.is_err());
}
