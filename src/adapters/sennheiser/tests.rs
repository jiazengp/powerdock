use super::*;
#[test]
fn exact_service_namespace_required() {
    assert!(gaia_service_node(
        r"BTHENUM\{A2129FF3-081B-4C45-8AFE-469D9C4842EC}_VID&0000"
    ));
    for id in [
        "headset",
        r"BTHENUM\{a2129ff3-081b-4c45-8afe-469d9c4842eb}_x",
        r"OTHER\{a2129ff3-081b-4c45-8afe-469d9c4842ec}_x",
    ] {
        assert!(!gaia_service_node(id));
    }
}
#[test]
fn identity_uses_wire_model_and_firmware_not_display_names() {
    let i = identity(1, b"M4AEBT Denim\0", &[0, 3, 0, 38, 0, 3]).unwrap();
    assert_eq!(i.product, Product::Momentum4);
    assert_eq!(i.firmware, Version::new(3, 38, 3));
    assert!(i.permitted(false).is_ok());
    let ac = identity(1, b"ACAEBT Black", &[0, 3, 0, 10, 0, 0]).unwrap();
    assert!(ac.permitted(false).is_err());
    assert!(ac.permitted(true).is_ok());
    let tw = identity(1, b"MTW3", &[0, 2, 0, 2, 0, 0]).unwrap();
    assert!(tw.permitted(true).is_err()); // Firmware capability gate, not just model membership.
    for model in [
        b"MOMENTUM 4".as_slice(),
        b"M4AEBTImpostor",
        b"M4AEBT\0junk",
        b"M4AEBT \xff",
    ] {
        assert!(identity(1, model, &[0, 3, 0, 38, 0, 3]).is_err());
    }
    assert!(identity(1, b"M4AEBT", &[3, 38, 3]).is_err());
}
#[test]
fn exact_readonly_requests_and_tws_components() {
    assert_eq!(
        request(sn::MODEL_ID_GET).unwrap(),
        [0xff, 3, 0, 0, 4, 0x95, 0x12, 6]
    );
    assert_eq!(
        request(sn::BATTERY_LEVEL_GET).unwrap(),
        [0xff, 3, 0, 0, 4, 0x95, 6, 3]
    );
    assert!(request(sn::FACTORY_RESET).is_err());
    let t = battery_observation(Product::MomentumTw4, &[80, 65, 0], Power::Charging).unwrap();
    assert_eq!(t.reading.value, Value::reported(65));
    assert_eq!(t.reading.power, Power::Unknown);
    assert_eq!(t.components.len(), 3);
    assert_eq!(t.components[2].role, "case");
    assert_eq!(t.components[2].value, Value::reported(0));
    let absent = battery_observation(Product::MomentumTw4, &[0, 255], Power::Battery).unwrap();
    assert_eq!(absent.reading.value, Value::Unknown);
    assert_eq!(absent.components.len(), 2);
    assert!(absent.components.iter().all(|c| c.value == Value::Unknown));
    assert!(battery_observation(Product::Momentum4, &[80, 60], Power::Unknown).is_err());
    assert!(battery_observation(Product::MomentumTw4, &[80], Power::Unknown).is_err());
    assert!(battery_observation(Product::MomentumTw4, &[80, 60, 50, 40], Power::Unknown).is_err());
}
#[test]
fn cancellation_before_connect_does_not_reuse_cached_authorization() {
    let mut cache = Some(identity(1, b"M4AEBT", &[0, 3, 0, 38, 0, 3]).unwrap());
    assert!(read(1, &AtomicU64::new(2), 1, &mut cache, false).is_err());
    let original = crate::settings::ProviderPreferences::default();
    let mut next = original.clone();
    next.enabled_experimental
        .insert(crate::settings::Profile::SennheiserFamilyExperimental);
    assert!(!original.profile(crate::settings::Profile::SennheiserFamilyExperimental));
    assert!(original.changed_for(&next, &[crate::battery::Source::Sennheiser]));
    assert!(!original.changed_for(&next, &[crate::battery::Source::Windows]));
}
#[test]
fn captured_fragmented_and_combined_replies() {
    let mut d = Decoder::default();
    d.push(&[0xff, 3, 0, 1, 4]).unwrap();
    assert!(d.next().unwrap().is_none());
    d.push(&[0x95, 7, 3, 100, 0xff, 3, 0, 2, 4, 0x95, 7, 2, 1, 0])
        .unwrap();
    let (cmd, p) = d.next().unwrap().unwrap();
    assert_eq!(cmd, 0x0703);
    assert_eq!(
        battery_observation(Product::Momentum4, &p, Power::Unknown)
            .unwrap()
            .reading
            .value,
        Value::reported(100)
    );
    assert_eq!(charging(&d.next().unwrap().unwrap().1), Power::Charging);
    assert!(d.next().unwrap().is_none());
}
#[test]
fn zero_unknown_and_checksum() {
    assert_eq!(
        battery_observation(Product::Momentum4, &[0], Power::Unknown)
            .unwrap()
            .reading
            .value,
        Value::reported(0)
    );
    assert_eq!(
        battery_observation(Product::Momentum4, &[255], Power::Unknown)
            .unwrap()
            .reading
            .value,
        Value::Unknown
    );
    assert_eq!(charging(&[0, 0]), Power::Battery);
    assert_eq!(charging(&[2, 0]), Power::Full);
    assert_eq!(charging(&[0, 1]), Power::Unknown);
    let mut bytes = vec![0xff, 3, 1, 1, 4, 0x95, 7, 3, 20];
    let sum = bytes.iter().fold(0, |s, b| s ^ b);
    bytes.push(sum);
    let mut d = Decoder::default();
    d.push(&bytes).unwrap();
    assert_eq!(
        battery_observation(
            Product::Momentum4,
            &d.next().unwrap().unwrap().1,
            Power::Unknown
        )
        .unwrap()
        .reading
        .value,
        Value::reported(20)
    );
    bytes[8] = 21;
    let mut d = Decoder::default();
    d.push(&bytes).unwrap();
    assert!(d.next().is_err());
}
