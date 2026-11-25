//! Read-only HID eligibility, using the production fingerprints and descriptors.
use crate::{
    adapters::{
        device::Device,
        hid_standard, hidpp,
        mchose::profiles::{self as mchose, MouseProtocol},
        steelseries,
    },
    settings::{Profile, ProviderPreferences, Reader},
};
use serde_json::{Value, json};

#[cfg(test)]
mod tests;

pub(super) fn inventory(devices: &[Device], policy_reply: (Value, i32)) -> Value {
    inventory_with(devices, policy_reply, |d| {
        hid_standard::inspect(d).map(|d| d.map(|d| d.feature_readable()))
    })
}

fn assessment(
    reader: Reader,
    profiles: &[Profile],
    policy: Option<&ProviderPreferences>,
    qualification: Option<&str>,
) -> Value {
    let enabled = policy.map(|p| p.reader(reader) && profiles.iter().all(|id| p.profile(*id)));
    let (stage, reason) = if let Some(reason) = qualification {
        ("qualification", reason)
    } else if let Some(p) = policy {
        if !p.reader(reader) {
            ("policy", "reader_disabled")
        } else if profiles.iter().any(|id| p.disabled_profiles.contains(id)) {
            ("policy", "profile_disabled")
        } else if enabled == Some(false) {
            ("policy", "experimental_opt_in_required")
        } else {
            ("qualification", "eligible_for_reader")
        }
    } else {
        ("policy", "policy_unavailable")
    };
    json!({"reader":reader.id(),"profiles":profiles,"stage":stage,"reason":reason,
        "configured_enabled":enabled,"hardware_verification":"not_checked"})
}

fn inventory_with(
    devices: &[Device],
    (reply, code): (Value, i32),
    mut descriptor: impl FnMut(&Device) -> Result<Option<bool>, String>,
) -> Value {
    // Never load/migrate settings here. Only a same-executable host can provide
    // the effective policy; an absent/stale host must not become default policy.
    let policy = (code == 0 && reply["ok"] == true)
        .then(|| {
            serde_json::from_value::<ProviderPreferences>(reply["data"]["preferences"].clone()).ok()
        })
        .flatten()
        .filter(ProviderPreferences::valid);
    let policy_status = if policy.is_some() {
        "running_host_snapshot"
    } else {
        "unavailable"
    };
    let policy_error = if policy.is_some() {
        Value::Null
    } else {
        json!(match reply["error"]["code"].as_str() {
            Some("not_running") => "not_running",
            Some("executable_mismatch") => "executable_mismatch",
            Some("transport_failure") => "transport_failure",
            _ => "invalid_policy_snapshot",
        })
    };
    let hidpp = hidpp::enumerate_from(devices);
    let steelseries = steelseries::enumerate_from(devices);
    let interfaces: Vec<_> = devices
        .iter()
        .map(|d| interface(d, policy.as_ref(), &hidpp, &steelseries, descriptor(d)))
        .collect();
    json!({"inventory_schema_version":2,"scope":"present_hid_interfaces","battery_protocol_queried":false,
        "standard_descriptors_inspected":true,"identifiers_redacted":true,"interfaces":interfaces,
        "policy":{"source":policy_status,"error_code":policy_error,"configuration_generation":if policy.is_some() {reply["data"]["configuration_generation"].clone()}else{Value::Null}},
        "note":"Reader eligibility is not a battery reading, connection proof, hardware verification or scheduling decision. Unclassified HID does not rule out Windows/Bluetooth support; interfaces unavailable to metadata enumeration are outside this inventory."})
}

fn interface(
    d: &Device,
    policy: Option<&ProviderPreferences>,
    hidpp: &[hidpp::Device],
    steelseries: &[Device],
    descriptor: Result<Option<bool>, String>,
) -> Value {
    let mut assessments = Vec::new();
    let mut compiled_profile = None;
    if let Some(protocol) = mchose::mouse_protocol(d) {
        let profile = match protocol {
            MouseProtocol::A7 { .. } => Profile::MchoseA7,
            MouseProtocol::Yjx { .. } => Profile::MchoseYjxExperimental,
        };
        compiled_profile = Some(if matches!(protocol, MouseProtocol::A7 { .. }) {
            "mchose_mouse"
        } else {
            "mchose_yjx_experimental"
        });
        assessments.push(assessment(
            if protocol.wired() {
                Reader::MchoseUsbHid
            } else {
                Reader::MchoseHid
            },
            &[profile],
            policy,
            None,
        ));
    }
    if mchose::mchose_keyboard(d) {
        let profiles = if mchose::sinowealth_keyboard(d) {
            compiled_profile = Some("mchose_sinowealth_experimental");
            vec![
                Profile::MchoseKeyboard,
                Profile::MchoseSinowealthExperimental,
            ]
        } else {
            compiled_profile = Some("mchose_keyboard");
            vec![Profile::MchoseKeyboard]
        };
        assessments.push(assessment(
            Reader::MchoseKeyboardHid,
            &profiles,
            policy,
            None,
        ));
    }
    if hidpp::eligible_collection(d) {
        compiled_profile = Some("logitech_hidpp_experimental");
        let path = String::from_utf16_lossy(&d.path)
            .trim_end_matches('\0')
            .to_owned();
        let candidates: Vec<_> = hidpp
            .iter()
            .filter(|candidate| candidate.matches_interface(std::slice::from_ref(&path)))
            .collect();
        let block = if candidates.is_empty() {
            Some("ambiguous_collections")
        } else if candidates.iter().any(|d| !d.monitor_eligible()) {
            Some("receiver_identity_unverified")
        } else {
            None
        };
        assessments.push(assessment(
            Reader::LogitechHidpp,
            &[Profile::LogitechHidppExperimental],
            policy,
            block,
        ));
    }
    if steelseries::profile(d).is_some() {
        compiled_profile = Some("steelseries_direct_experimental");
        let block = (!steelseries.contains(d)).then_some("ambiguous_collections");
        assessments.push(assessment(
            Reader::SteelseriesHid,
            &[Profile::SteelseriesDirectExperimental],
            policy,
            block,
        ));
    }
    let qualification = if compiled_profile.is_some() {
        "known_protocol_fingerprint"
    } else {
        "unclassified"
    };
    let standard = match descriptor {
        Ok(Some(readable)) => assessment(
            Reader::HidStandard,
            &[Profile::HidStandardExperimental],
            policy,
            (!readable).then_some("input_only_or_ambiguous_fields"),
        ),
        Ok(None) => {
            json!({"reader":"hid_standard","stage":"descriptor","reason":"no_qualified_battery_field","configured_enabled":null})
        }
        // Do not emit native error strings: future errors may contain paths.
        Err(error) => {
            json!({"reader":"hid_standard","stage":"descriptor","reason":match error.split(':').next().unwrap_or("") {
                "access_denied"=>"access_denied", "busy"=>"busy", "disconnected"=>"disconnected", _=>"descriptor_unavailable"
            },"configured_enabled":null})
        }
    };
    assessments.push(standard);
    json!({"vid":d.vendor_id,"pid":d.product_id,"usage_page":d.usage_page,"usage":d.usage,
            "input_bytes":d.input_bytes,"output_bytes":d.output_bytes,"compiled_profile":compiled_profile,
            "qualification":qualification,"assessments":assessments})
}
