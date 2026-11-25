use super::*;
#[test]
fn identity_is_case_insensitive_and_source_specific() {
    assert_eq!(key("os", "ABC"), key("os", "abc"));
    assert_ne!(key("os", "abc"), key("hid", "abc"));
    assert_ne!(key("os", "abc"), key("os", "abd"));
}
#[test]
fn detected_device_without_battery_keeps_its_type_and_never_becomes_lowest() {
    let device = Tracked::update(
        Sample {
            connection: None,
            id: "headset".into(),
            name: "headset".into(),
            source: Source::Windows,
            kind: Kind::Headset,
            at: 100,
            clock: std::time::Instant::now(),
            identity: crate::identity::Evidence::Unverified,
            session: 0,
            reader_epoch: 0,
            value_observation: None,
            components: Vec::new(),
            result: Err("connected, no battery property".into()),
        },
        None,
    );
    assert_eq!(device.kind, Kind::Headset);
    assert!(device.last.is_none());
    assert_eq!(lowest(&[device], None, 100), None);
}
#[test]
fn unavailable_device_does_not_inherit_another_reading() {
    let a = Tracked::update(
        Sample {
            connection: None,
            id: "a".into(),
            name: "mouse".into(),
            source: Source::Mchose,
            kind: Kind::Mouse,
            at: 100,
            clock: std::time::Instant::now(),
            identity: crate::identity::Evidence::Unverified,
            session: 0,
            reader_epoch: 0,
            value_observation: None,
            components: Vec::new(),
            result: Ok(Reading {
                value: crate::battery::Value::reported(40),
                kind: Kind::Mouse,
                power: Power::Battery,
                connected: true,
            }),
        },
        None,
    );
    let stale = Tracked::update(
        Sample {
            connection: None,
            id: "a".into(),
            name: "mouse".into(),
            source: Source::Mchose,
            kind: Kind::Mouse,
            at: 200,
            clock: std::time::Instant::now(),
            identity: crate::identity::Evidence::Unverified,
            session: 0,
            reader_epoch: 0,
            value_observation: None,
            components: Vec::new(),
            result: Err("offline".into()),
        },
        Some(&a),
    );
    assert_eq!(stale.last.unwrap().1, 100);
    assert!(stale.error.is_some());
    let new = Tracked::update(
        Sample {
            connection: None,
            id: "b".into(),
            name: "mouse".into(),
            source: Source::Mchose,
            kind: Kind::Mouse,
            at: 200,
            clock: std::time::Instant::now(),
            identity: crate::identity::Evidence::Unverified,
            session: 0,
            reader_epoch: 0,
            value_observation: None,
            components: Vec::new(),
            result: Err("offline".into()),
        },
        None,
    );
    assert!(new.last.is_none());
}
