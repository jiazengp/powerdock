use super::*;

fn reply(policy: ProviderPreferences) -> (Value, i32) {
    (
        json!({"ok":true,"data":{"preferences":policy,"configuration_generation":7}}),
        0,
    )
}
fn mouse(pid: u16, wired: bool) -> Device {
    Device {
        path: "private-device-path\0".encode_utf16().collect(),
        container: Some("private-container".into()),
        product: "private-product-name".into(),
        vendor_id: 0x3837,
        product_id: pid,
        usage_page: 0xff01,
        usage: if pid == 0x4031 { 1 } else { 0x10 },
        input_bytes: if wired && pid == 0x4031 { 64 } else { 65 },
        output_bytes: if wired && pid == 0x4031 { 64 } else { 65 },
    }
}
fn reason(report: &Value, interface: usize, assessment: usize) -> &str {
    report["interfaces"][interface]["assessments"][assessment]["reason"]
        .as_str()
        .unwrap()
}

#[test]
fn policy_precedence_matches_per_device_production_rules() {
    for (pid, wired, reader, profile) in [
        (0x4031, true, Reader::MchoseUsbHid, Profile::MchoseA7),
        (
            0x418c,
            true,
            Reader::MchoseUsbHid,
            Profile::MchoseYjxExperimental,
        ),
        (
            0x4245,
            false,
            Reader::MchoseHid,
            Profile::MchoseYjxExperimental,
        ),
    ] {
        let d = mouse(pid, wired);
        let mut p = ProviderPreferences::default();
        for step in 0..4 {
            match step {
                1 => {
                    if profile.experimental() {
                        p.enabled_experimental.insert(profile);
                    }
                }
                2 => {
                    p.disabled_profiles.insert(profile);
                }
                3 => {
                    p.disabled_readers.insert(reader);
                }
                _ => {}
            }
            let report = inventory_with(std::slice::from_ref(&d), reply(p.clone()), |_| Ok(None));
            let result = &report["interfaces"][0]["assessments"][0];
            assert_eq!(result["configured_enabled"], p.mouse_device_enabled(&d));
            assert_eq!(result["reader"], reader.id());
            assert_eq!(
                reason(&report, 0, 0),
                if step == 3 {
                    "reader_disabled"
                } else if step == 2 {
                    "profile_disabled"
                } else if profile.experimental() && step == 0 {
                    "experimental_opt_in_required"
                } else {
                    "eligible_for_reader"
                }
            );
        }
    }
}

#[test]
fn sinowealth_requires_base_keyboard_profile_as_well_as_opt_in() {
    let mut d = mouse(0x2004, false);
    d.vendor_id = 0x41e4;
    d.usage_page = 0xff02;
    d.usage = 2;
    d.input_bytes = 20;
    d.output_bytes = 20;
    let mut p = ProviderPreferences::default();
    p.enabled_experimental
        .insert(Profile::MchoseSinowealthExperimental);
    p.disabled_profiles.insert(Profile::MchoseKeyboard);
    assert!(!p.keyboard_device_enabled(&d));
    let report = inventory_with(&[d], reply(p), |_| Ok(None));
    assert_eq!(reason(&report, 0, 0), "profile_disabled");
}

#[test]
fn missing_or_invalid_host_policy_is_unknown_not_defaults() {
    for host_reply in [
        (json!({"ok":false,"error":{"code":"not_running"}}), 4),
        (
            json!({"ok":true,"data":{"preferences":{"unexpected":true}}}),
            0,
        ),
        (
            json!({"ok":true,"data":{"preferences":{"enabled_experimental":["mchose_a7"]}}}),
            0,
        ),
    ] {
        let report = inventory_with(&[mouse(0x4031, true)], host_reply, |_| Ok(Some(true)));
        assert_eq!(report["policy"]["source"], "unavailable");
        assert!(report["policy"]["configuration_generation"].is_null());
        for a in report["interfaces"][0]["assessments"].as_array().unwrap() {
            assert_eq!(a["reason"], "policy_unavailable");
            assert!(a["configured_enabled"].is_null());
        }
    }
}

#[test]
fn standard_descriptor_failures_are_separate_from_missing_fields_and_policy() {
    for (probe, expected) in [
        (Ok(None), "no_qualified_battery_field"),
        (Ok(Some(false)), "input_only_or_ambiguous_fields"),
        (Ok(Some(true)), "experimental_opt_in_required"),
        (
            Err("access_denied: private-device-path".into()),
            "access_denied",
        ),
        (Err("busy: private-device-path".into()), "busy"),
        (
            Err("disconnected: private-device-path".into()),
            "disconnected",
        ),
        (
            Err("unsupported: private-device-path".into()),
            "descriptor_unavailable",
        ),
    ] {
        let report = inventory_with(
            &[mouse(0xffff, false)],
            reply(ProviderPreferences::default()),
            |_| probe.clone(),
        );
        assert_eq!(reason(&report, 0, 0), expected);
        assert_eq!(report["interfaces"][0]["qualification"], "unclassified");
        assert!(!report.to_string().contains("private-"));
    }
}

#[test]
fn steelseries_ambiguity_cannot_be_reported_as_eligible() {
    let a = steelseries::tests::interface("secret", 0x1890);
    let mut b = a.clone();
    b.path = "another&mi_03\0".encode_utf16().collect();
    let mut p = ProviderPreferences::default();
    p.enabled_experimental
        .insert(Profile::SteelseriesDirectExperimental);
    let single = inventory_with(std::slice::from_ref(&a), reply(p.clone()), |_| Ok(None));
    assert_eq!(reason(&single, 0, 0), "eligible_for_reader");
    let ambiguous = inventory_with(&[a, b], reply(p), |_| Ok(None));
    for i in 0..2 {
        assert_eq!(reason(&ambiguous, i, 0), "ambiguous_collections");
    }
}

#[test]
fn hidpp_receiver_pause_and_ambiguous_routes_use_real_grouping() {
    let mut d = mouse(0xc52b, false);
    d.vendor_id = 0x046d;
    d.usage_page = 0xff00;
    d.usage = 1;
    d.input_bytes = 7;
    d.output_bytes = 7;
    let mut p = ProviderPreferences::default();
    p.enabled_experimental
        .insert(Profile::LogitechHidppExperimental);
    let report = inventory_with(std::slice::from_ref(&d), reply(p.clone()), |_| Ok(None));
    assert_eq!(reason(&report, 0, 0), "receiver_identity_unverified");
    let mut duplicate = d.clone();
    duplicate.path = "another\0".encode_utf16().collect();
    let report = inventory_with(&[d, duplicate], reply(p), |_| Ok(None));
    assert_eq!(reason(&report, 0, 0), "ambiguous_collections");
}

#[test]
fn inventory_redacts_identifiers_and_keeps_legacy_fields_without_claiming_reads() {
    let report = inventory_with(
        &[mouse(0x4031, true)],
        reply(ProviderPreferences::default()),
        |_| Ok(None),
    );
    assert!(!report.to_string().contains("private-"));
    assert_eq!(report["inventory_schema_version"], 2);
    assert_eq!(report["battery_protocol_queried"], false);
    assert_eq!(report["policy"]["configuration_generation"], 7);
    assert_eq!(report["interfaces"][0]["compiled_profile"], "mchose_mouse");
    assert_eq!(
        report["interfaces"][0]["qualification"],
        "known_protocol_fingerprint"
    );
    assert_eq!(
        report["interfaces"][0]["assessments"][0]["hardware_verification"],
        "not_checked"
    );
}

#[test]
fn disabling_primary_keeps_standard_descriptor_and_policy_independent() {
    let device = steelseries::tests::interface("separate-sources", 0x1890);
    let mut policy = ProviderPreferences::default();
    policy.enabled_experimental.extend([
        Profile::SteelseriesDirectExperimental,
        Profile::HidStandardExperimental,
    ]);
    for primary_disabled in [true, false] {
        if primary_disabled {
            policy.disabled_readers.insert(Reader::SteelseriesHid);
        } else {
            policy.disabled_readers.remove(&Reader::SteelseriesHid);
            policy
                .disabled_profiles
                .insert(Profile::SteelseriesDirectExperimental);
        }
        let report = inventory_with(std::slice::from_ref(&device), reply(policy.clone()), |_| {
            Ok(Some(true))
        });
        assert_eq!(
            reason(&report, 0, 0),
            if primary_disabled {
                "reader_disabled"
            } else {
                "profile_disabled"
            }
        );
        assert_eq!(reason(&report, 0, 1), "eligible_for_reader");
        assert_eq!(
            report["interfaces"][0]["assessments"][1]["configured_enabled"],
            true
        );
    }
}
