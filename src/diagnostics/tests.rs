use super::bundle::export_from;
use super::privacy::replace_case_insensitive;
use super::*;
#[test]
fn battery_snapshot_exports_only_observation_fields_not_identity_evidence() {
    let exported = snapshot(json!({"devices":[{"name":"Private device","battery":{
        "schema_version":1,"value_kind":"ordinal","level":"low","display":"Private label","serial":"private-serial","endpoint":"private-endpoint",
        "components":[{"id":"main","role":"main","name":"Private component","value":{"value_kind":"reported_percent","percent":0,"unit_id":"private-unit"}}]
    }}]}));
    assert_eq!(exported["devices"][0]["battery"]["level"], "low");
    assert_eq!(
        exported["devices"][0]["battery"]["components"][0]["value"]["percent"],
        0
    );
    assert!(!exported.to_string().to_lowercase().contains("private"));
}
#[test]
fn removes_profile_paths_and_device_names() {
    assert_eq!(
        replace_case_insensitive("C:\\Users\\Test\\x", "c:\\users\\test", "<user>"),
        "<user>\\x"
    );
    let state = snapshot(
        json!({"devices":[{"id":"mouse-123","name":"Personal device","path":"secret","address":"secret","percent":0}]}),
    );
    assert!(state["devices"][0].get("name").is_none());
    assert_eq!(state["devices"][0]["percent"], 0);
}
#[test]
fn recent_records_are_bounded_redacted_and_newest_first() {
    let scratch = std::env::temp_dir().join(format!("powerdock-recent-{}", std::process::id()));
    fs::create_dir_all(&scratch).unwrap();
    let log = scratch.join("powerdock.2026-11-22.jsonl");
    let mut lines = String::new();
    for index in 0..(MAX_RECENT_LOG + 2) {
        lines.push_str(&format!(
            "{}\n",
            json!({"timestamp":format!("2026-11-22T10:00:{index:02}Z"),"level":"INFO",
                "fields":{"event":format!("event_{index}"),"details":json!({"request_id":index,"id":"route-1"}).to_string()}})
        ));
    }
    fs::write(&log, lines).unwrap();
    let records = recent_at(&scratch, false);
    let records = records.as_array().unwrap();
    assert_eq!(records.len(), MAX_RECENT_LOG);
    assert_eq!(records[0]["event"], format!("event_{}", MAX_RECENT_LOG + 1));
    assert_eq!(records[0]["details"], "request_id=7 · id=route-1");
    fs::remove_dir_all(&scratch).unwrap();
}
#[test]
fn recent_summary_reads_nested_fields_and_unwraps_operations() {
    assert_eq!(
        summarize_log_details(
            &json!({"component":"window","child_pid":42,
                "data":{"event":"settings_tab","details":{"page":1}}})
            .to_string()
        ),
        "component=window · child_pid=42 · page=1"
    );
    assert_eq!(
        summarize_log_details(
            &json!({"operation":{"menu":false,"operation":"show","settings":true},"request_id":2})
                .to_string()
        ),
        "operation=show · request_id=2"
    );
    assert_eq!(
        summarize_log_details(&json!({"error":null,"request_id":3}).to_string()),
        "request_id=3"
    );
}
#[test]
fn bundle_is_parseable_and_never_overwrites_existing_files() {
    let scratch = std::env::temp_dir().join(format!(
        "powerdock-diagnostics-{}-{}",
        std::process::id(),
        next_request()
    ));
    fs::create_dir_all(&scratch).unwrap();
    fs::write(
        scratch.join("powerdock.2026-11-15.jsonl"),
        "{\"event\":\"test\"}\n",
    )
    .unwrap();
    fs::write(scratch.join("settings.json"), "private").unwrap();
    let target = scratch.join("report.zip");
    export_from(&scratch, &target, json!({"devices":[]})).unwrap();
    let mut archive = zip::ZipArchive::new(File::open(&target).unwrap()).unwrap();
    assert_eq!(archive.len(), 3);
    let mut text = String::new();
    archive
        .by_name("manifest.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap()["schema_version"],
        1
    );
    assert!(export_from(&scratch, &target, Value::Null).is_err());
    drop(archive);
    fs::remove_dir_all(scratch).unwrap();
}
