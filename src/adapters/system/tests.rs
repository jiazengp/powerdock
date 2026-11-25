use super::reader::{read_connected, standard_hid_fallback};
use super::*;
#[test]
fn paired_catalog_absence_does_not_erase_present_classic_brand_candidates() {
    let headset = Peripheral::test_classic_headset("any-display-name");
    let mut old = Peripheral::test_paired("old-phone", false);
    old.kind = Kind::Phone;
    let mut devices = vec![headset.clone(), old];
    retain_paired_candidates(&mut devices, &[]);
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].id, headset.id);
    assert!(!devices[0].connected_hint); // Retention is not a connection assertion.
    let result: Result<(), _> = read_connected([Some(false), None], |_| {
        panic!("Offline history must never query a protocol")
    });
    assert!(result.is_err());
}
#[test]
fn hid_fallback_is_independent_of_bt_connection_and_preserves_better_sources() {
    for connected in [[Some(false), Some(false)], [None, None]] {
        let mut source = Source::Windows;
        let mut connection = connected[0];
        let bt = read_connected(connected, |_| panic!("disconnected BT must not query"));
        let result = standard_hid_fallback(bt, &mut source, &mut connection, || {
            Ok(Reading::new(
                crate::battery::Value::reported(42),
                Kind::Mouse,
                Power::Unknown,
            ))
        })
        .unwrap();
        assert_eq!(result.percent(), Some(42));
        assert_eq!(source, Source::HidStandard);
        assert_eq!(connection, Some(true));
    }
    let mut source = Source::BluetoothGatt;
    let mut connection = Some(true);
    let reading = Reading::new(
        crate::battery::Value::reported(12),
        Kind::Mouse,
        Power::Battery,
    );
    assert_eq!(
        standard_hid_fallback(
            Ok(reading.clone()),
            &mut source,
            &mut connection,
            || panic!("do not replace direct GATT")
        )
        .unwrap()
        .percent(),
        Some(12)
    );
    source = Source::Windows;
    assert_eq!(
        standard_hid_fallback(Ok(reading), &mut source, &mut connection, || Err(
            "access_denied: exclusive".into()
        ))
        .unwrap()
        .percent(),
        Some(12)
    );
    assert_eq!(source, Source::Windows);
    assert_eq!(
        container_event_key("{00112233-4455-6677-8899-AABBCCDDEEFF}"),
        Some("33221100554477668899aabbccddeeff".into())
    );
    assert_eq!(container_event_key("not-a-container"), None);
}
#[test]
fn typed_charging_overlay_is_independent_of_public_battery_percentage() {
    for percent in [None, Some(101), Some(255), Some(40)] {
        let meta = MetadataBattery::from_properties(percent, Some(1));
        assert_eq!(
            meta.power_or(|| panic!("known charging must override stale legacy state")),
            Power::Charging
        );
    }
    for charging in [0, 2, 255] {
        let meta = MetadataBattery::from_properties(None, Some(charging));
        assert_eq!(meta.power_or(|| Power::Charging), Power::Unknown);
    }
    let absent = MetadataBattery::from_properties(Some(40), None);
    assert_eq!(absent.power_or(|| Power::Battery), Power::Battery);
}
#[test]
fn classic_headset_policy_scopes_sdp_discovery_without_name_matching() {
    let mut classic = discovered(false, Kind::Headset, true, 1);
    assert_eq!(classic.sources(), vec![Source::Windows, Source::Sennheiser]);
    classic.name = "MOMENTUM 4".into();
    assert_eq!(classic.sources(), vec![Source::Windows, Source::Sennheiser]); // Names are never protocol evidence.
    assert_eq!(
        discovered(false, Kind::Mouse, true, 3).sources(),
        vec![Source::Windows]
    );
    let mut le = discovered(true, Kind::Headset, true, 2);
    le.name = "MOMENTUM 4".into();
    assert_eq!(le.sources(), vec![Source::Windows, Source::BluetoothGatt]);
}
fn discovered(le: bool, kind: Kind, connected: bool, node: u32) -> Peripheral {
    let endpoint = Endpoint {
        address: 0xaabbccddeeff,
        connected_hint: connected,
        address_type: le.then_some(BluetoothAddressType::Random),
    };
    Peripheral {
        standard_hid: None,
        id: battery::key("bt", "aabbccddeeff"),
        name: "dual-mode peripheral".into(),
        container: Some("physical-device".into()),
        kind,
        connected_hint: connected,
        nodes: if node == 0 { Vec::new() } else { vec![node] },
        classic: (!le).then_some(endpoint.clone()),
        low_energy: le.then_some(endpoint),
    }
}
#[test]
fn same_mac_keeps_both_transports_and_their_metadata_in_either_order() {
    for le_first in [false, true] {
        let mut devices = Vec::new();
        merge_device(&mut devices, discovered(le_first, Kind::Other, false, 1));
        merge_device(&mut devices, discovered(!le_first, Kind::Headset, false, 2));
        // Paired AEP supplementation has no PnP nodes but must retain its LE endpoint.
        merge_device(&mut devices, discovered(true, Kind::Other, true, 0));
        assert_eq!(devices.len(), 1);
        let device = &devices[0];
        assert_eq!(device.nodes, [1, 2]);
        assert_eq!(device.kind, Kind::Headset);
        assert!(!device.classic.as_ref().unwrap().connected_hint);
        assert!(device.low_energy.as_ref().unwrap().connected_hint);
        assert_eq!(device.classic.as_ref().unwrap().address, 0xaabbccddeeff);
        assert_eq!(
            device.low_energy.as_ref().unwrap().address_type,
            Some(BluetoothAddressType::Random)
        );
        assert!(device.connected_hint);
    }
    let mut aep_only = Vec::new();
    merge_device(&mut aep_only, discovered(false, Kind::Other, false, 0));
    merge_device(&mut aep_only, discovered(true, Kind::Mouse, true, 0));
    assert_eq!(aep_only.len(), 1);
    assert!(aep_only[0].classic.is_some());
    assert!(aep_only[0].low_energy.is_some());
}
#[test]
fn unavailable_classic_does_not_mask_live_le_battery_service() {
    for classic in [Some(false), None] {
        let result = read_connected([classic, Some(true)], |le| {
            assert!(le, "offline or unknown Classic must not be read");
            Ok(73)
        });
        assert_eq!(result, Ok(73));
    }
    let mut attempts = Vec::new();
    let result = read_connected([Some(true), Some(true)], |le| {
        attempts.push(le);
        if le {
            Ok(73)
        } else {
            Err("no Classic battery metadata".into())
        }
    });
    assert_eq!(result, Ok(73));
    assert_eq!(attempts, [false, true]);
    let result = read_connected([Some(true), Some(true)], |le| {
        assert!(!le, "available Classic cache retains its existing priority");
        Ok(42)
    });
    assert_eq!(result, Ok(42));
}
#[test]
fn aep_hints_cannot_authorize_a_battery_read_without_live_connection() {
    let hinted = discovered(true, Kind::Mouse, true, 0);
    assert!(hinted.connected_hint);
    for statuses in [
        [None, None],
        [Some(false), Some(false)],
        [Some(false), None],
    ] {
        let result: Result<u8, String> = read_connected(statuses, |_| {
            panic!("neither endpoint has a confirmed live connection");
        });
        assert!(result.is_err());
    }
}
#[test]
fn endpoint_identity_is_strict_and_matches_pnp_guid_byte_order() {
    assert_eq!(endpoint_address("AA:BB:CC:DD:EE:FF"), Some(0xaabbccddeeff));
    assert_eq!(endpoint_address("aa-bb-cc-dd-ee-ff"), Some(0xaabbccddeeff));
    assert_eq!(endpoint_address("000000000000"), None);
    assert_eq!(endpoint_address("aabbccddeeff00"), None);
    assert_eq!(endpoint_address("host-aabbccddeeff"), None);
    assert_eq!(guid_key(windows::core::GUID::zeroed()), None);
    assert_eq!(
        guid_key(windows::core::GUID::from_u128(
            0x01234567_89ab_cdef_0123_456789abcdef
        )),
        Some("67452301ab89efcd0123456789abcdef".into())
    );
}
#[test]
fn bluetooth_identity_requires_real_root_not_an_arbitrary_hex_suffix() {
    assert_eq!(
        bluetooth_address("BTHLE\\DEV_AABBCCDDEEFF\\x"),
        Some((0xaabbccddeeff, true))
    );
    assert_eq!(
        bluetooth_address("BTHENUM\\DEV_AABBCCDDEEFF\\x"),
        Some((0xaabbccddeeff, false))
    );
    assert_eq!(bluetooth_address("HID\\SOMETHING_AABBCCDDEEFF"), None);
    assert_eq!(bluetooth_address("BTHLE\\DEV_AABBCCDDEEFF00\\x"), None);
}
#[test]
fn numeric_property_types_and_sizes_are_checked() {
    assert_eq!(integer(DEVPROP_TYPE_BYTE, &[0]), Some(0));
    assert_eq!(
        integer(DEVPROP_TYPE_UINT32, &100u32.to_le_bytes()),
        Some(100)
    );
    assert_eq!(integer(DEVPROP_TYPE_BOOLEAN, &[1]), None);
    assert_eq!(integer(DEVPROP_TYPE_UINT32, &[20]), None);
}
