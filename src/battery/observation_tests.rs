use super::*;
fn sample(value: Value, source: Source, at: u64) -> Sample {
    Sample {
        id: "device".into(),
        name: "Device".into(),
        source,
        kind: Kind::Mouse,
        connection: Some(true),
        at,
        clock: Instant::now(),
        identity: crate::identity::Evidence::Unverified,
        session: 1,
        reader_epoch: 0,
        value_observation: None,
        components: Vec::new(),
        result: Ok(Reading {
            value,
            kind: Kind::Mouse,
            power: Power::Battery,
            connected: true,
        }),
    }
}
#[test]
fn precision_presence_and_zero_do_not_invent_percentages_or_alerts() {
    for value in [
        Value::Unknown,
        Value::NoBattery,
        Value::Ordinal {
            level: Level::Critical,
        },
        Value::EstimatedPercent {
            percent: 10,
            model_version: 1,
        },
        Value::reported(255),
    ] {
        let d = Tracked::update(sample(value, Source::Mchose, 100), None);
        assert_eq!(d.reading(100).and_then(Reading::percent), None);
        assert!(lowest(std::slice::from_ref(&d), None, 100).is_none());
        assert!(!crate::settings::AlertMemory::default().eligible(
            d.reading(100).unwrap(),
            20,
            100
        ));
        assert!(d.presentation(100).get("value_kind").is_some());
    }
    let zero = Tracked::update(sample(Value::reported(0), Source::Mchose, 100), None);
    assert_eq!(zero.reading(100).and_then(Reading::percent), Some(0));
    assert_eq!(zero.presentation(100)["presence"], "present");
    let absent = Tracked::update(sample(Value::NoBattery, Source::Mchose, 100), None);
    assert_eq!(absent.presentation(100)["presence"], "absent");
}
#[test]
fn unchanged_os_cache_even_with_power_change_does_not_renew_value_time() {
    let first = Tracked::update(sample(Value::reported(40), Source::Windows, 100), None);
    let mut next = sample(Value::reported(40), Source::Windows, 350);
    next.result.as_mut().unwrap().power = Power::Charging;
    let repeated = Tracked::update(next, Some(&first));
    assert_eq!(repeated.last.as_ref().unwrap().1, 100);
    assert_eq!(repeated.received_at, Some(350));
    assert_eq!(repeated.value_changed_at, Some(100));
    assert_eq!(repeated.last.as_ref().unwrap().0.power, Power::Charging);
    assert!(repeated.reading(401).is_none());
    let changed = Tracked::update(
        sample(Value::reported(39), Source::Windows, 402),
        Some(&repeated),
    );
    assert_eq!(changed.value_changed_at, Some(402));
    assert!(changed.reading(402).is_some());
    assert!(changed.presentation(402)["measured_at"].is_null());
}
#[test]
fn expired_cached_source_cannot_mask_a_fresh_canonical_fallback() {
    let mut cached = sample(Value::reported(40), Source::BluetoothGatt, 1_299);
    cached.value_observation = Some((1_000, Instant::now() - Duration::from_secs(301)));
    let fallback = sample(Value::reported(39), Source::Windows, 1_300);
    for reversed in [false, true] {
        let mut samples = vec![cached.clone(), fallback.clone()];
        if reversed {
            samples.reverse();
        }
        retain_preferred_samples(&mut samples, 1_301);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].source, Source::Windows);
        assert_eq!(
            Tracked::update(samples.remove(0), None)
                .reading(1_301)
                .unwrap()
                .percent(),
            Some(39)
        );
    }
}
#[test]
fn cached_battery_expiry_does_not_expire_a_new_live_connection_check() {
    for source in [Source::Windows, Source::BluetoothGatt] {
        let packet_clock = Instant::now() - Duration::from_secs(299);
        let mut old = sample(Value::reported(40), source, 1_000);
        old.clock = packet_clock;
        let previous = Tracked::update(old, None);
        let mut checked = sample(Value::reported(40), source, 1_299);
        checked.value_observation = Some((1_000, packet_clock));
        let current = Tracked::update(checked, Some(&previous));
        assert_eq!(current.observed_at, 1_299);
        assert_eq!(current.received_at, Some(1_299));
        assert_eq!(current.last.as_ref().unwrap().1, 1_000);
        assert!(current.visible(1_301));
        assert!(current.reading(1_301).is_none());
        assert_eq!(current.presentation(1_301)["attempted_at"], 1_299);
        let mut changed = sample(Value::reported(39), source, 1_299);
        changed.value_observation = Some((1_000, packet_clock));
        let current = Tracked::update(changed, None);
        assert!(current.visible(1_301));
        assert!(current.reading(1_301).is_none());
    }
}
#[test]
fn clock_rollback_and_snapshot_replays_cannot_revive_old_observations() {
    let mut old = sample(Value::reported(40), Source::Mchose, 1000);
    old.clock = Instant::now() - Duration::from_secs(301);
    let tracked = Tracked::update(old.clone(), None);
    assert!(tracked.reading(900).is_none());
    let replay = Tracked::update(old, Some(&tracked));
    assert!(replay.reading(900).is_none());
    let fresh = Tracked::update(
        sample(Value::reported(40), Source::Mchose, 900),
        Some(&replay),
    );
    assert!(fresh.reading(900).is_some());
}
#[test]
fn precision_groups_keep_their_order_when_reversed_and_tray_uses_levels_only_as_fallback() {
    use crate::settings::{SortBy, SortOrder};
    let values = [
        Value::Unknown,
        Value::EstimatedPercent {
            percent: 1,
            model_version: 1,
        },
        Value::Ordinal { level: Level::Low },
        Value::reported(90),
        Value::reported(10),
    ];
    let devices: Vec<_> = values
        .into_iter()
        .enumerate()
        .map(|(i, v)| {
            let mut s = sample(v, Source::Mchose, 100);
            s.id = format!("device-{i}");
            Tracked::update(s, None)
        })
        .collect();
    for (order, ids) in [
        (
            SortOrder::Asc,
            ["device-4", "device-3", "device-2", "device-1", "device-0"],
        ),
        (
            SortOrder::Desc,
            ["device-3", "device-4", "device-2", "device-1", "device-0"],
        ),
    ] {
        assert_eq!(
            ordered(&devices, SortBy::Battery, order, 100)
                .iter()
                .map(|d| d.id.as_str())
                .collect::<Vec<_>>(),
            ids
        );
    }
    assert_eq!(lowest(&devices, None, 100), Some("device-4".into()));
    assert_eq!(lowest_level(&devices, None, 100), Some("device-2".into()));
}
