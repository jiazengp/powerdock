use super::*;

pub(super) fn interface(container: &str, pid: u16, usage: u16) -> device::Device {
    let bytes = if usage == 1 { 7 } else { 20 };
    device::Device {
        path: format!("synthetic-{container}-{usage}\0")
            .encode_utf16()
            .collect(),
        container: Some(container.into()),
        vendor_id: 0x046d,
        product_id: pid,
        product: "Synthetic HID++ peripheral".into(),
        usage_page: 0xff00,
        usage,
        input_bytes: bytes,
        output_bytes: bytes,
    }
}
#[test]
fn collection_discovery_is_strict_and_two_report_routes_share_one_candidate() {
    let short = interface("direct", 0xb001, 1);
    let long = interface("direct", 0xb001, 2);
    let devices = enumerate_from(&[short.clone(), long.clone()]);
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].slot, 0xff);
    assert!(devices[0].monitor_eligible());
    assert_eq!(devices[0].endpoints.len(), 2);
    assert!(devices[0].matches_interface(&["SYNTHETIC-DIRECT-1".into()]));
    let mut invalid = short.clone();
    invalid.vendor_id = 0x1234;
    assert!(enumerate_from(&[invalid]).is_empty());
    let mut invalid = short.clone();
    invalid.usage_page = 0xff43;
    assert!(enumerate_from(&[invalid]).is_empty());
    let mut invalid = long;
    invalid.output_bytes = 64;
    assert!(enumerate_from(&[invalid]).is_empty());
    assert!(enumerate_from(&[short.clone(), short]).is_empty());
}
#[test]
fn receiver_slots_are_distinct_but_paused_without_replacement_identity_evidence() {
    let mut devices = enumerate_from(&[
        interface("receiver-one", 0xc52b, 1),
        interface("receiver-two", 0xc52b, 1),
    ]);
    assert_eq!(devices.len(), 12);
    assert_eq!(
        devices
            .iter()
            .map(Device::gate_key)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        2
    );
    assert_eq!(
        devices
            .iter()
            .map(|d| d.id.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        12
    );
    for d in &devices {
        assert!(!d.monitor_eligible());
        assert!(d.is_receiver());
        assert!(matches!(Session::open(d),Err(e) if e==RECEIVER_PAUSED));
    }
    // An arbitrary pairing replacement cannot inherit an old route's settings:
    // the entire unverified receiver path remains ineligible before any GET.
    devices[0].name = "Same model replacement".into();
    assert!(!devices[0].monitor_eligible());
}
