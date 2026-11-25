//! Build-time checks shared by the Rust build and its regression tests.
use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

fn placeholders(template: &str) -> Result<BTreeSet<&str>, String> {
    let mut names = BTreeSet::new();
    let mut remaining = template;
    while let Some((_, tail)) = remaining.split_once("%{") {
        let (name, rest) = tail
            .split_once('}')
            .ok_or_else(|| "unclosed placeholder".to_owned())?;
        let mut bytes = name.bytes();
        if !bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
            || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(format!("invalid placeholder %{{{name}}}"));
        }
        names.insert(name);
        remaining = rest;
    }
    Ok(names)
}

pub fn validate_catalogs(catalogs: &[(&str, Value)]) -> Result<(), String> {
    let mut baseline = None;
    for (language, value) in catalogs {
        let catalog = value
            .as_object()
            .ok_or_else(|| format!("{language}: expected a JSON object"))?;
        if catalog.get("_version").and_then(Value::as_u64) != Some(1) {
            return Err(format!("{language}: _version must be 1"));
        }
        let mut entries = BTreeMap::new();
        for (key, value) in catalog.iter().filter(|(key, _)| *key != "_version") {
            let text = value
                .as_str()
                .filter(|text| !text.trim().is_empty())
                .ok_or_else(|| format!("{language}/{key}: expected a nonempty string"))?;
            let names = placeholders(text).map_err(|error| format!("{language}/{key}: {error}"))?;
            entries.insert(key.as_str(), names);
        }
        if entries.is_empty() {
            return Err(format!("{language}: catalog has no translations"));
        }
        if let Some((baseline_language, baseline_entries)) = &baseline {
            let baseline_entries: &BTreeMap<&str, BTreeSet<&str>> = baseline_entries;
            let missing: Vec<_> = baseline_entries
                .keys()
                .filter(|key| !entries.contains_key(**key))
                .collect();
            let extra: Vec<_> = entries
                .keys()
                .filter(|key| !baseline_entries.contains_key(**key))
                .collect();
            if !missing.is_empty() || !extra.is_empty() {
                return Err(format!(
                    "{language}: keys differ from {baseline_language}; missing {missing:?}, extra {extra:?}"
                ));
            }
            for (key, names) in &entries {
                if names != &baseline_entries[key] {
                    return Err(format!(
                        "{language}/{key}: placeholders {names:?} differ from {baseline_language} {:?}",
                        baseline_entries[key]
                    ));
                }
            }
        } else {
            baseline = Some((*language, entries));
        }
    }
    if baseline.is_none() {
        return Err("no locale catalogs provided".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_reordered_repeated_placeholders_and_literal_percent() {
        validate_catalogs(&[
            (
                "en-US",
                json!({"_version": 1, "Battery": "%{name}: %{percent}%"}),
            ),
            (
                "zh-CN",
                json!({"_version": 1, "Battery": "%{percent}% · %{name} %{name}"}),
            ),
        ])
        .unwrap();
    }

    #[test]
    fn rejects_invalid_templates_with_locale_and_key() {
        for template in ["%{", "%{name", "%{}", "%{na me}", "%{0name}", "%{%{name}}"] {
            let error =
                validate_catalogs(&[("zh-CN", json!({"_version": 1, "Battery": template}))])
                    .unwrap_err();
            assert!(error.starts_with("zh-CN/Battery:"), "{error}");
        }
    }

    #[test]
    fn rejects_incomplete_or_invalid_catalogs() {
        let baseline = json!({"_version": 1, "Battery": "%{name}: %{percent}%"});
        for invalid in [
            json!({"_version": 1, "Other": "text"}),
            json!({"_version": 1, "Battery": "%{name}"}),
            json!({"_version": 1, "Battery": "   "}),
            json!({"_version": 1, "Battery": 20}),
            json!({"_version": 2, "Battery": "%{name}: %{percent}%"}),
            json!({"Battery": "%{name}: %{percent}%"}),
            json!({"_version": 1}),
            json!([]),
        ] {
            let error =
                validate_catalogs(&[("en-US", baseline.clone()), ("zh-CN", invalid)]).unwrap_err();
            assert!(
                error.starts_with("zh-CN:") || error.starts_with("zh-CN/"),
                "{error}"
            );
        }
    }
}
