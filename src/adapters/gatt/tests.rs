use super::characteristic::accept_notification;
use super::*;
use crate::battery::{Level, Power, Value};
use std::sync::Mutex;
#[test]
fn optional_bls_fields_truncation_and_rfu_are_bounded() {
    for flags in 0..8u8 {
        let mut data = vec![flags, 1, 0];
        if flags & 1 != 0 {
            data.extend_from_slice(&0x0106u16.to_le_bytes());
        }
        if flags & 2 != 0 {
            data.push(100);
        }
        if flags & 4 != 0 {
            data.push(0);
        }
        assert!(status(&data).is_ok());
        for n in 0..data.len() {
            assert!(status(&data[..n]).is_err());
        }
        data[0] |= 0xf8;
        data.extend_from_slice(&[0xff, 0xff]);
        assert!(status(&data).is_ok());
    }
    let s = status(&[0, 1 | 0x80, 1]).unwrap();
    assert_eq!(
        component_value(None, Some(s), None).unwrap().0,
        Value::Ordinal {
            level: Level::Critical
        }
    );
}
#[test]
fn rejected_subscription_polls_and_successful_subscription_has_a_health_deadline() {
    let now = Instant::now();
    assert!(requires_read(false, Some(now), now));
    assert!(requires_read(true, None, now));
    assert!(!requires_read(
        true,
        Some(now),
        now + Duration::from_secs(299)
    ));
    assert!(requires_read(
        true,
        Some(now),
        now + Duration::from_secs(300)
    ));
}
#[test]
fn retired_notification_cannot_replace_cached_data() {
    let epoch = AtomicU64::new(1);
    let cache = Mutex::new(None);
    assert!(accept_notification(&cache, &epoch, 1, vec![10]));
    let original = cache.lock().unwrap().as_ref().unwrap().clock;
    epoch.store(2, Ordering::Release);
    assert!(!accept_notification(&cache, &epoch, 1, vec![0]));
    assert_eq!(cache.lock().unwrap().as_ref().unwrap().bytes, vec![10]);
    assert_eq!(cache.lock().unwrap().as_ref().unwrap().clock, original);
}
#[test]
fn levels_are_exact_bounded_bytes() {
    for v in [0, 100] {
        assert_eq!(level(&[v]), Ok(v));
    }
    for data in [vec![], vec![101], vec![255], vec![0, 1]] {
        assert!(level(&data).is_err());
    }
}
#[test]
fn bls_flags_presence_and_power_never_invent_full() {
    for charge in 0..4 {
        let raw = 1 | (charge << 5);
        let data = [2, raw, 0, 40];
        let s = status(&data).unwrap();
        assert!(s.present);
        assert_eq!(
            s.power,
            [
                Power::Unknown,
                Power::Charging,
                Power::Battery,
                Power::Inactive
            ][charge as usize]
        );
        assert_eq!(s.level, Some(40));
    }
    let s = status(&[2, 0, 0, 0]).unwrap();
    assert_eq!(
        component_value(Some(0), Some(s), None).unwrap().0,
        Value::NoBattery
    );
    for data in [
        vec![],
        vec![0],
        vec![0, 1],
        vec![1, 1, 0, 0],
        vec![2, 1, 0, 101],
        vec![2, 1, 0, 255],
        vec![4, 1, 0],
    ] {
        assert!(status(&data).is_err());
    }
    assert!(status(&[0x80, 1, 0, 99]).is_ok());
    assert_eq!(status(&[0, 1 | 4, 0]).unwrap().external, None);
}
#[test]
fn cpf_identifier_conflicts_and_instance_reordering_are_checked() {
    let desc = [4, 0, 0xad, 0x27, 1, 0x0d, 1];
    assert_eq!(cpf(&desc), Ok(0x010d));
    assert!(cpf(&desc[..6]).is_err());
    let s = status(&[3, 1, 0, 0x0d, 1, 30]).unwrap();
    assert_eq!(
        component_value(Some(31), Some(s), Some(0x010d)).unwrap().0,
        Value::reported(30)
    );
    assert_eq!(
        component_value(Some(29), Some(s), Some(0x010d)).unwrap().0,
        Value::reported(30)
    );
    assert!(component_value(Some(30), Some(s), Some(0x010e)).is_err());
    let a = Component {
        id: "bas-010d".into(),
        role: "unknown",
        value: Value::reported(20),
        power: Power::Unknown,
        external_power: None,
        identifier: Some(0x010d),
        raw_power_state: None,
    };
    let mut b = a.clone();
    b.id = "bas-010e".into();
    b.value = Value::reported(80);
    assert_eq!(
        aggregate(&[a.clone(), b.clone()], Kind::Headset).unwrap(),
        aggregate(&[b, a.clone()], Kind::Headset).unwrap()
    );
    assert!(aggregate(&[a.clone(), a], Kind::Headset).is_err());
}
