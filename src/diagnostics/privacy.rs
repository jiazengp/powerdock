//! Diagnostics whitelist projection and local path/identifier redaction.
use super::*;

pub fn sanitize(text: &str) -> String {
    let mut value = text.to_owned();
    for name in ["USERPROFILE", "LOCALAPPDATA", "APPDATA"] {
        if let Some(path) = std::env::var_os(name) {
            let path = path.to_string_lossy();
            value = replace_case_insensitive(&value, &path, &format!("<{name}>"));
            value =
                replace_case_insensitive(&value, &path.replace('\\', "/"), &format!("<{name}>"));
        }
    }
    // Native HID/Bluetooth paths are not needed to correlate already-opaque device IDs.
    static DEVICE_PATH: OnceLock<regex::Regex> = OnceLock::new();
    let pattern = DEVICE_PATH.get_or_init(|| {
        regex::Regex::new(r#"(?i)(?:hid#|bthenum\\)[^\s\"<>]*"#)
            .expect("valid device redaction pattern")
    });
    pattern.replace_all(&value, "<device-path>").into_owned()
}
pub(super) fn replace_case_insensitive(text: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return text.into();
    }
    let lower = text.to_ascii_lowercase();
    let find = needle.to_ascii_lowercase();
    let mut result = String::new();
    let mut at = 0;
    while let Some(index) = lower[at..].find(&find) {
        let index = at + index;
        result.push_str(&text[at..index]);
        result.push_str(replacement);
        at = index + needle.len();
    }
    result.push_str(&text[at..]);
    result
}
pub fn sanitized_value(mut value: Value) -> Value {
    fn visit(value: &mut Value) {
        match value {
            Value::String(text) => *text = sanitize(text),
            Value::Array(items) => items.iter_mut().for_each(visit),
            Value::Object(object) => object.values_mut().for_each(visit),
            _ => {}
        }
    }
    visit(&mut value);
    value
}
pub fn snapshot(mut state: Value) -> Value {
    if let Some(object) = state.as_object_mut() {
        object.remove("device_name");
    }
    if let Some(devices) = state.get_mut("devices").and_then(Value::as_array_mut) {
        for device in devices {
            if let Some(object) = device.as_object_mut() {
                object.remove("name");
                object.remove("path");
                object.remove("address");
                if let Some(battery) = object.get_mut("battery") {
                    retain_fields(
                        battery,
                        &[
                            "schema_version",
                            "value_kind",
                            "percent",
                            "resolution_percent",
                            "level",
                            "model_version",
                            "precision",
                            "presence",
                            "power",
                            "fresh",
                            "freshness",
                            "source_cache",
                            "source",
                            "connection",
                            "evidence_scope",
                            "external_power",
                            "activity",
                            "attempted_at",
                            "received_at",
                            "last_success_at",
                            "value_changed_at",
                            "measured_at",
                            "components",
                        ],
                    );
                    if let Some(components) =
                        battery.get_mut("components").and_then(Value::as_array_mut)
                    {
                        for component in components {
                            retain_fields(
                                component,
                                &[
                                    "id",
                                    "role",
                                    "value",
                                    "power",
                                    "measured_at",
                                    "identifier",
                                    "external_power",
                                    "raw_power_state",
                                    "observed_at",
                                    "fresh",
                                    "source",
                                ],
                            );
                            if let Some(value) = component.get_mut("value") {
                                retain_fields(
                                    value,
                                    &[
                                        "value_kind",
                                        "percent",
                                        "resolution_percent",
                                        "level",
                                        "model_version",
                                    ],
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    sanitized_value(state)
}
fn retain_fields(value: &mut Value, fields: &[&str]) {
    if let Some(object) = value.as_object_mut() {
        object.retain(|key, _| fields.contains(&key.as_str()));
    }
}
