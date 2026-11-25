use super::*;
fn device(id: &str, percent: u8, at: u64) -> Tracked {
    Tracked::update(
        Sample {
            connection: None,
            id: id.into(),
            name: id.into(),
            source: Source::Mchose,
            kind: Kind::Mouse,
            at,
            clock: std::time::Instant::now(),
            identity: crate::identity::Evidence::Unverified,
            session: 0,
            reader_epoch: 0,
            value_observation: None,
            components: Vec::new(),
            result: Ok(Reading {
                value: Value::reported(percent),
                kind: Kind::Mouse,
                power: Power::Battery,
                connected: true,
            }),
        },
        None,
    )
}
#[test]
fn battery_tone_uses_fresh_state_and_charging_takes_priority() {
    for (percent, power, expected) in [
        (0, Power::Battery, BatteryTone::Low),
        (20, Power::Battery, BatteryTone::Low),
        (21, Power::Battery, BatteryTone::Normal),
        (100, Power::Full, BatteryTone::Normal),
        (60, Power::Unknown, BatteryTone::Normal),
        (3, Power::Charging, BatteryTone::Charging),
    ] {
        let mut current = device("mouse", percent, 100);
        current.last.as_mut().unwrap().0.power = power;
        assert_eq!(current.battery_tone(100), expected);
        assert_eq!(current.battery_tone(401), BatteryTone::Unknown);
        current.source = Source::Windows;
        assert_eq!(current.battery_tone(100), BatteryTone::Unknown);
        current.source = Source::Mchose;
        current.error = Some("receiver unavailable".into());
        assert_eq!(current.battery_tone(100), BatteryTone::Unknown);
    }
    let mut disconnected = device("mouse", 50, 100);
    disconnected.last.as_mut().unwrap().0.connected = false;
    assert_eq!(disconnected.battery_tone(100), BatteryTone::Unknown);
}
#[test]
fn disconnected_and_unconfirmed_routes_hide_but_connected_unknown_battery_stays() {
    let previous = device("mouse", 40, 100);
    assert!(previous.visible(100));
    for (connection, visible) in [(Some(false), false), (None, false), (Some(true), true)] {
        let current = Tracked::update(
            Sample {
                id: "mouse".into(),
                name: "mouse".into(),
                source: Source::Windows,
                kind: Kind::Mouse,
                connection,
                at: 101,
                clock: Instant::now(),
                identity: crate::identity::Evidence::Unverified,
                session: 0,
                reader_epoch: 0,
                value_observation: None,
                components: Vec::new(),
                result: Err("battery unavailable".into()),
            },
            Some(&previous),
        );
        assert_eq!(current.visible(101), visible);
        assert_eq!(lowest(std::slice::from_ref(&current), None, 101), None);
        assert!(!current.visible(402));
    }
}
#[test]
fn list_battery_order_reverses_known_values_but_unknown_is_always_last() {
    use crate::settings::{SortBy, SortOrder};
    let mut unknown = device("unknown", 1, 100);
    unknown.error = Some("unavailable".into());
    let rows = vec![
        device("high", 80, 100),
        unknown,
        device("zero", 0, 100),
        device("tie-b", 80, 100),
    ];
    let ids = |order| {
        ordered(&rows, SortBy::Battery, order, 100)
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(SortOrder::Asc),
        vec!["zero", "high", "tie-b", "unknown"]
    );
    assert_eq!(
        ids(SortOrder::Desc),
        vec!["high", "tie-b", "zero", "unknown"]
    );
    assert_eq!(lowest(&rows, None, 100), Some("zero".into()));
}
#[test]
fn list_groups_types_and_power_without_changing_the_tray_minimum() {
    use crate::settings::{SortBy, SortOrder};
    let mut rows = Vec::new();
    for (id, kind, power) in [
        ("m", Kind::Mouse, Power::Battery),
        ("k", Kind::Keyboard, Power::Charging),
        ("h", Kind::Headset, Power::Full),
        ("g", Kind::Gamepad, Power::Unknown),
    ] {
        let mut row = device(id, 50, 100);
        row.kind = kind;
        let reading = &mut row.last.as_mut().unwrap().0;
        reading.kind = kind;
        reading.power = power;
        rows.push(row);
    }
    let ids = |by, order| {
        ordered(&rows, by, order, 100)
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(SortBy::Type, SortOrder::Asc), vec!["m", "k", "h", "g"]);
    assert_eq!(ids(SortBy::Type, SortOrder::Desc), vec!["g", "h", "k", "m"]);
    assert_eq!(
        ids(SortBy::Charging, SortOrder::Asc),
        vec!["k", "m", "h", "g"]
    );
    assert_eq!(
        ids(SortBy::Charging, SortOrder::Desc),
        vec!["h", "m", "k", "g"]
    );
}
#[test]
fn lowest_and_ties_are_stable() {
    let a = device("a", 25, 100);
    let b = device("b", 10, 100);
    assert_eq!(
        lowest(&[a.clone(), b.clone()], Some("a"), 100),
        Some("b".into())
    );
    let c = device("c", 10, 100);
    assert_eq!(lowest(&[b, c], Some("c"), 100), Some("c".into()));
}
#[test]
fn stale_offline_and_failed_readings_are_excluded() {
    let a = device("a", 1, 1);
    let mut b = device("b", 2, 400);
    b.error = Some("offline".into());
    let c = device("c", 50, 400);
    assert_eq!(lowest(&[a, b, c], None, 400), Some("c".into()));
    assert_eq!(lowest(&[], None, 400), None);
}
#[test]
fn zero_is_valid_and_charging_is_not_silently_excluded() {
    let a = device("a", 0, 100);
    let mut b = device("b", 5, 100);
    b.last.as_mut().unwrap().0.power = Power::Charging;
    assert_eq!(lowest(&[a.clone(), b.clone()], None, 100), Some("a".into()));
    assert_eq!(lowest(&[b], None, 100), Some("b".into()));
}
