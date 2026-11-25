use super::*;

fn field(min: i32, max: i32, bits: u16, has_null: bool) -> Field {
    Field {
        report_type: ReportType::Feature,
        report_id: 7,
        report_bytes: 3,
        link: 4,
        bits,
        min,
        max,
        has_null,
    }
}
fn percent(value: Value) -> (u8, Option<u8>) {
    match value {
        Value::ReportedPercent {
            percent,
            resolution_percent,
        } => (percent, resolution_percent),
        _ => panic!("expected reported percentage"),
    }
}
#[test]
fn exact_percent_keeps_zero_and_hundred_and_rejects_101() {
    let f = field(0, 100, 8, false);
    for raw in [0, 50, 100] {
        assert_eq!(
            percent(f.value(raw, HIDP_STATUS_SUCCESS).unwrap()).0,
            raw as u8
        );
    }
    assert!(f.value(101, HIDP_STATUS_SUCCESS).is_err());
    assert!(f.value(255, HIDP_STATUS_SUCCESS).is_err());
}
#[test]
fn coarse_four_steps_remain_ordinal() {
    let f = field(0, 3, 2, false);
    for (raw, level) in [
        (0, Level::Critical),
        (1, Level::Low),
        (2, Level::Medium),
        (3, Level::Full),
    ] {
        assert_eq!(
            f.value(raw, HIDP_STATUS_SUCCESS).unwrap(),
            Value::Ordinal { level }
        );
    }
}
#[test]
fn signed_fields_are_extended_before_scaling() {
    let f = field(-100, 0, 8, false);
    assert_eq!(
        percent(f.value(156, HIDP_STATUS_SUCCESS).unwrap()),
        (0, Some(1))
    );
    assert_eq!(
        percent(f.value(206, HIDP_STATUS_SUCCESS).unwrap()),
        (50, Some(1))
    );
    assert_eq!(
        percent(f.value(0, HIDP_STATUS_SUCCESS).unwrap()),
        (100, Some(1))
    );
    assert!(f.value(1, HIDP_STATUS_SUCCESS).is_err());
    let wide = field(i32::MIN, i32::MAX, 32, false);
    assert_eq!(
        percent(wide.value(0x80000000, HIDP_STATUS_SUCCESS).unwrap()).0,
        0
    );
}
#[test]
fn null_is_descriptor_defined_and_255_can_be_valid() {
    let null = field(0, 100, 8, true);
    assert_eq!(
        null.value(255, HIDP_STATUS_SUCCESS).unwrap(),
        Value::Unknown
    );
    assert_eq!(null.value(0, HIDP_STATUS_NULL).unwrap(), Value::Unknown);
    assert!(field(0, 100, 8, false).value(0, HIDP_STATUS_NULL).is_err());
    assert_eq!(
        percent(
            field(0, 255, 8, false)
                .value(255, HIDP_STATUS_SUCCESS)
                .unwrap()
        ),
        (100, Some(1))
    );
}
#[test]
fn invalid_ranges_widths_and_parser_errors_never_clamp() {
    for f in [
        field(5, 5, 8, false),
        field(10, 0, 8, false),
        field(0, 100, 0, false),
        field(0, 100, 33, false),
        field(0, 100, 2, false),
    ] {
        assert!(f.value(0, HIDP_STATUS_SUCCESS).is_err());
    }
    assert!(
        field(0, 100, 8, false)
            .value(256, HIDP_STATUS_SUCCESS)
            .is_err()
    );
    assert!(
        field(0, 100, 8, false)
            .value(0, HIDP_STATUS_USAGE_NOT_FOUND)
            .is_err()
    );
    assert_eq!(
        percent(
            field(0, 20, 8, false)
                .value(10, HIDP_STATUS_SUCCESS)
                .unwrap()
        ),
        (50, Some(5))
    );
}
#[test]
fn short_long_and_wrong_id_reports_are_rejected_before_parsing() {
    let f = field(0, 100, 8, false);
    assert!(f.validate_report(&[7, 50]).is_err());
    assert!(f.validate_report(&[7, 50, 0, 0]).is_err());
    assert!(f.validate_report(&[8, 50, 0]).is_err());
    assert!(f.validate_report(&[7, 50, 0]).is_ok());
    let zero_id = Field { report_id: 0, ..f };
    assert!(zero_id.validate_report(&[0, 50, 0]).is_ok());
}
#[test]
fn descriptor_matching_preserves_type_id_link_range_and_rejects_arrays() {
    let mut cap = HIDP_VALUE_CAPS {
        UsagePage: PAGE,
        ReportID: 9,
        BitField: 2,
        LinkCollection: 12,
        IsAbsolute: true,
        BitSize: 8,
        ReportCount: 1,
        LogicalMin: 0,
        LogicalMax: 100,
        ..Default::default()
    };
    cap.Anonymous.NotRange.Usage = USAGE;
    let feature = Field::from_cap(&cap, ReportType::Feature, 3).unwrap();
    assert_eq!(
        (feature.report_id, feature.link, feature.min, feature.max),
        (9, 12, 0, 100)
    );
    assert_ne!(
        Field::from_cap(&cap, ReportType::Input, 3).unwrap(),
        feature
    );
    cap.ReportCount = 2;
    assert!(Field::from_cap(&cap, ReportType::Feature, 3).is_none());
    cap.IsRange = true;
    cap.Anonymous.Range.UsageMin = 0x1f;
    cap.Anonymous.Range.UsageMax = 0x21;
    assert!(Field::from_cap(&cap, ReportType::Feature, 3).is_some());
    cap.Anonymous.Range.UsageMin = 0x21;
    assert!(Field::from_cap(&cap, ReportType::Feature, 3).is_none());
}
#[test]
fn input_only_and_multiple_feature_devices_never_open_a_transport() {
    let mut device = Device {
        id: "test".into(),
        name: "test".into(),
        container: None,
        associated: false,
        kind: Kind::Keyboard,
        path: Vec::new(),
        fields: vec![Field {
            report_type: ReportType::Input,
            ..field(0, 100, 8, false)
        }],
    };
    let generation = AtomicU64::new(1);
    assert!(!device.feature_readable());
    assert!(
        device
            .query(&generation, 1)
            .unwrap_err()
            .starts_with("unsupported:")
    );
    device.fields = vec![field(0, 100, 8, false), field(0, 3, 2, false)];
    assert!(
        device
            .query(&generation, 1)
            .unwrap_err()
            .starts_with("unsupported:")
    );
    assert!(
        device
            .query(&generation, 0)
            .unwrap_err()
            .starts_with("cancelled:")
    );
}
#[test]
fn debug_never_exposes_device_paths() {
    let device = Device {
        id: "opaque".into(),
        name: "test".into(),
        container: None,
        associated: false,
        kind: Kind::Other,
        path: "private-hardware-address".encode_utf16().collect(),
        fields: vec![],
    };
    assert!(!format!("{device:?}").contains("private-hardware-address"));
}

#[test]
fn transport_errors_do_not_mislabel_every_failure_as_access_denied() {
    for (code, prefix) in [
        (Some(0x80070005u32 as i32), "access_denied:"),
        (Some(0x80070020u32 as i32), "busy:"),
        (Some(0x8007048fu32 as i32), "disconnected:"),
        (Some(0x80070002u32 as i32), "disconnected:"),
        (Some(0), "unsupported:"),
        (None, "unsupported:"),
    ] {
        assert!(async_error(code).starts_with(prefix));
    }
}
