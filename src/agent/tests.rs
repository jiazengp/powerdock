use super::cli::Cli;
use super::*;
use clap::Parser;
use serde_json::json;
fn candidates(count: usize) -> Vec<crate::monitor::Candidate> {
    (0..count)
        .map(|i| crate::monitor::Candidate {
            id: format!("candidate-{i}"),
            name: "中文候选".repeat(12),
            kind: crate::battery::Kind::Mouse,
            sources: vec![crate::battery::Source::Mchose],
            scheduling: if i < crate::battery::MAX_DEVICES {
                "active"
            } else {
                "budget_deferred"
            },
        })
        .collect()
}
#[test]
fn catalog_pages_preserve_all_candidates_and_reject_mixed_revisions() {
    let mut entries = candidates(96);
    let mut cursor = None;
    let mut ids = Vec::new();
    loop {
        let response = catalog_page(&entries, cursor.as_deref(), 32);
        assert_eq!(response["ok"], true);
        assert!(serde_json::to_vec(&response).unwrap().len() <= MAX_BYTES);
        ids.extend(
            response["data"]["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["id"].as_str().unwrap().to_owned()),
        );
        cursor = response["data"]["next_cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(
        ids,
        entries.iter().map(|c| c.id.clone()).collect::<Vec<_>>()
    );
    let first = catalog_page(&entries, None, 1);
    let cursor = first["data"]["next_cursor"].as_str().unwrap();
    assert_eq!(first, catalog_page(&entries, None, 1));
    entries[0].scheduling = "budget_deferred";
    assert_eq!(
        catalog_page(&entries, Some(cursor), 1)["error"]["code"],
        "stale_revision"
    );
    for size in [0, 33] {
        assert_eq!(catalog_page(&entries, None, size)["ok"], false);
    }
    for cursor in [
        "bad".to_string(),
        format!("{}.999", first["data"]["revision"].as_str().unwrap()),
        "0.0".into(),
    ] {
        assert_eq!(
            catalog_page(&candidates(96), Some(&cursor), 1)["error"]["code"],
            "invalid_cursor"
        );
    }
    let empty = catalog_page(&[], None, 32);
    assert_eq!(empty["data"]["total"], 0);
    assert!(empty["data"]["next_cursor"].is_null());
}
#[test]
fn catalog_pages_measure_serialized_utf8_bytes_including_json_escapes() {
    let mut entries = candidates(4);
    for c in &mut entries {
        c.name = "中\0".repeat(2000);
    }
    let first = catalog_page(&entries, None, 32);
    assert_eq!(first["ok"], true);
    let count = first["data"]["candidates"].as_array().unwrap().len();
    assert!((1..4).contains(&count));
    assert!(serde_json::to_vec(&first).unwrap().len() <= MAX_BYTES);
    assert_eq!(first["data"]["candidates"][0]["name"], entries[0].name);
    let second = catalog_page(&entries, first["data"]["next_cursor"].as_str(), 32);
    assert_eq!(second["data"]["offset"], count);
    entries[0].name = "\0".repeat(MAX_BYTES);
    assert_eq!(
        catalog_page(&entries, None, 32)["error"]["code"],
        "response_too_large"
    );
}
#[test]
fn provider_cli_requires_explicit_state_and_compiled_ids() {
    for args in [
        vec!["agent", "providers", "reader", "--reader", "mchose-hid"],
        vec![
            "agent",
            "providers",
            "reader",
            "--reader",
            "anything",
            "--enabled",
            "false",
        ],
    ] {
        assert!(Cli::try_parse_from(args).is_err());
    }
    assert!(
        Cli::try_parse_from([
            "agent",
            "providers",
            "reader",
            "--reader",
            "mchose-hid",
            "--enabled",
            "false"
        ])
        .is_ok()
    );
    assert!(
        serde_json::from_str::<Operation>(
            r#"{"operation":"provider_reader_set","reader":"unknown","enabled":false}"#
        )
        .is_err()
    );
}
#[test]
fn general_preferences_are_typed_and_strict() {
    assert!(matches!(
        serde_json::from_str::<Operation>(
            r#"{"operation":"general_set","language":"en-US","preview_updates":true}"#
        )
        .unwrap(),
        Operation::GeneralSet { .. }
    ));
    assert!(
        serde_json::from_str::<Operation>(r#"{"operation":"general_set","language":"xx"}"#)
            .is_err()
    );
    assert!(
        serde_json::from_str::<Operation>(r#"{"operation":"general_set","unexpected":true}"#)
            .is_err()
    );
    assert_eq!(
        run(&[
            "settings".into(),
            "general".into(),
            "--language".into(),
            "xx".into()
        ]),
        2
    );
}
#[test]
fn popup_transparency_cli_and_ipc_share_bounds() {
    for value in ["0", "20", "50", "100"] {
        assert!(
            Cli::try_parse_from([
                "agent",
                "settings",
                "general",
                "--overlay-transparency",
                value
            ])
            .is_ok()
        );
        let patch: GeneralPatch = serde_json::from_value(
            serde_json::json!({"overlay_transparency":value.parse::<u8>().unwrap()}),
        )
        .unwrap();
        assert!(patch.validate().is_ok());
        assert!(!patch.empty());
    }
    for value in ["-1", "101", "255", "256", "1.5", "NaN"] {
        assert!(
            Cli::try_parse_from([
                "agent",
                "settings",
                "general",
                "--overlay-transparency",
                value
            ])
            .is_err()
        );
    }
    let patch: GeneralPatch = serde_json::from_str(r#"{"overlay_transparency":101}"#).unwrap();
    assert!(patch.validate().is_err());
    assert!(GeneralPatch::default().validate().is_err());
    let bad_duration: GeneralPatch = serde_json::from_str(r#"{"overlay_duration":7}"#).unwrap();
    assert!(bad_duration.validate().is_err());
}
#[test]
fn winrt_menu_json_roundtrips_to_the_typed_reminder_operation() {
    use windows::{
        Data::Json::{JsonObject, JsonValue},
        Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
        core::HSTRING,
    };
    let apartment = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    assert!(apartment.is_ok());
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    let _apartment = Apartment;
    for threshold in [None, Some(0), Some(10), Some(20), Some(30)] {
        let operation = JsonObject::new().unwrap();
        operation
            .Insert(
                &HSTRING::from("operation"),
                &JsonValue::CreateStringValue(&HSTRING::from("device_reminder_set")).unwrap(),
            )
            .unwrap();
        operation
            .Insert(
                &HSTRING::from("id"),
                &JsonValue::CreateStringValue(&HSTRING::from("mchose-test")).unwrap(),
            )
            .unwrap();
        let value = match threshold {
            Some(value) => JsonValue::CreateNumberValue(f64::from(value)),
            None => JsonValue::CreateNullValue(),
        }
        .unwrap();
        operation
            .Insert(&HSTRING::from("threshold"), &value)
            .unwrap();
        let bytes = operation.Stringify().unwrap().to_string();
        assert!(bytes.len() <= 256);
        match serde_json::from_str::<Operation>(&bytes).unwrap() {
            Operation::DeviceReminderSet {
                id,
                threshold: decoded,
            } => {
                assert_eq!(id, "mchose-test");
                assert_eq!(decoded, threshold);
            }
            _ => panic!("Incorrect menu operation"),
        }
    }
}
#[test]
fn clap_rejects_invalid_or_empty_mutations() {
    assert!(
        Cli::try_parse_from([
            "agent",
            "settings",
            "general",
            "--overlay-position",
            "bottom-left",
            "--overlay-monitor",
            "primary",
            "--overlay-duration",
            "10"
        ])
        .is_ok()
    );
    for duration in ["3", "6", "10", "15", "30"] {
        assert!(
            Cli::try_parse_from([
                "agent",
                "settings",
                "general",
                "--overlay-duration",
                duration
            ])
            .is_ok()
        );
    }
    for duration in ["0", "1", "2", "7", "20", "31"] {
        assert!(
            Cli::try_parse_from([
                "agent",
                "settings",
                "general",
                "--overlay-duration",
                duration
            ])
            .is_err()
        );
    }
    for style in ["windows", "overlay", "both"] {
        assert!(
            Cli::try_parse_from([
                "agent",
                "settings",
                "general",
                "--notification-style",
                style,
                "--notification-sound",
                "false"
            ])
            .is_ok()
        );
    }
    assert!(
        Cli::try_parse_from([
            "agent",
            "settings",
            "general",
            "--notification-style",
            "invalid"
        ])
        .is_err()
    );
    assert!(Cli::try_parse_from(["agent", "notification-test"]).is_ok());
    for args in [
        vec!["agent", "settings", "set"],
        vec!["agent", "settings", "set", "--threshold", "101"],
        vec!["agent", "settings", "set", "--interval", "1"],
        vec!["agent", "settings", "set", "--sort", "invalid"],
        vec!["agent", "settings", "set", "--order", "invalid"],
        vec!["agent", "devices", "refresh", "--id", "../config"],
    ] {
        assert!(Cli::try_parse_from(args).is_err());
    }
    assert!(
        Cli::try_parse_from([
            "agent",
            "settings",
            "set",
            "--threshold",
            "0",
            "--startup",
            "false"
        ])
        .is_ok()
    );
    assert!(
        Cli::try_parse_from([
            "agent", "settings", "set", "--sort", "type", "--order", "desc"
        ])
        .is_ok()
    );
}
#[test]
fn reminder_cli_requires_one_explicit_choice_and_a_real_opaque_id() {
    for arguments in [
        vec!["agent", "devices", "reminder", "set", "--id", "mouse"],
        vec![
            "agent",
            "devices",
            "reminder",
            "set",
            "--id",
            "mouse",
            "--inherit",
            "--threshold",
            "10",
        ],
        vec![
            "agent",
            "devices",
            "reminder",
            "set",
            "--id",
            "mouse",
            "--threshold",
            "101",
        ],
        vec!["agent", "devices", "reminder", "get", "--id", "../mouse"],
    ] {
        assert!(Cli::try_parse_from(arguments).is_err());
    }
    assert!(
        Cli::try_parse_from([
            "agent",
            "devices",
            "reminder",
            "set",
            "--id",
            "mouse",
            "--inherit"
        ])
        .is_ok()
    );
    assert!(
        Cli::try_parse_from([
            "agent",
            "devices",
            "reminder",
            "set",
            "--id",
            "mouse",
            "--threshold",
            "0"
        ])
        .is_ok()
    );
}
#[test]
fn wire_protocol_rejects_unknown_fields_and_operations() {
    assert!(
        serde_json::from_str::<Request>(
            r#"{"version":1,"command":{"operation":"quit","unexpected":true}}"#
        )
        .is_err()
    );
    assert!(
        serde_json::from_str::<Request>(r#"{"version":1,"command":{"operation":"write_hid"}}"#)
            .is_err()
    );
    let value = success(json!({"percent":null}));
    assert!(value["data"]["percent"].is_null());
    assert_eq!(value["schema_version"], 1);
}
