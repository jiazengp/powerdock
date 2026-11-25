use super::*;
use std::ptr::null_mut;
#[test]
fn yjx_cancel_and_deadline_precede_native_submission() {
    let no_device = Handle(null_mut()); // If the guard is missing this produces an I/O error, not the expected cancellation.
    let epoch = AtomicU64::new(2);
    let error = query_yjx_packet(
        &no_device,
        yjx::Command::Battery,
        &epoch,
        1,
        Instant::now() + Duration::from_secs(1),
    )
    .unwrap_err();
    assert_eq!(error, "设备会话已变更");
    let error = query_yjx_packet(
        &no_device,
        yjx::Command::Battery,
        &epoch,
        2,
        Instant::now() - Duration::from_millis(1),
    )
    .unwrap_err();
    assert_eq!(error, "电量查询超时");
}
#[test]
fn a7_accepts_its_known_transports_but_rejects_unrelated_collections() {
    let mut device = Device {
        path: vec![0],
        container: None,
        vendor_id: 0x3837,
        product_id: 0x4031,
        product: "MCHOSE A7 V3 Pro".into(),
        usage_page: 0xff01,
        usage: 1,
        input_bytes: 64,
        output_bytes: 64,
    };
    for pid in [0x4031, 0x1014, 0x1018] {
        device.product_id = pid;
        assert!(profiles::mouse_protocol(&device).is_some());
    }
    device.product_id = 0x4032;
    assert!(profiles::mouse_protocol(&device).is_none());
    device.product_id = 0x4031;
    device.product = "Renamed mouse".into();
    assert!(profiles::mouse_protocol(&device).is_some());
    device.product = "MCHOSE A7 V3 Pro".into();
    device.usage_page = 0xff0b;
    assert!(profiles::mouse_protocol(&device).is_none());
}
