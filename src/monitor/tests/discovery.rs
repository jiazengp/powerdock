use super::*;
use crate::monitor::worker::transfer_link_observation;

#[test]
fn yjx_routes_share_policy_source_and_identity_transport() {
    let mut d = device::Device {
        path: vec![],
        container: Some("physical".into()),
        vendor_id: 0x3837,
        product_id: 0x4245,
        product: "Any display name".into(),
        usage_page: 0xff01,
        usage: 0x10,
        input_bytes: 65,
        output_bytes: 65,
    };
    let mut p = settings::ProviderPreferences::default();
    let rf = Target::Mchose(d.clone());
    assert!(!rf.direct_reader_enabled(&p));
    p.enabled_experimental
        .insert(settings::Profile::MchoseYjxExperimental);
    assert!(rf.direct_reader_enabled(&p));
    assert_eq!(rf.sample().source, Source::Mchose);
    assert!(matches!(rf.sample().identity, Evidence::Route(_)));
    d.product_id = 0x418c;
    let wired = Target::Mchose(d);
    assert_eq!(wired.sample().source, Source::MchoseUsb);
    assert!(matches!(wired.sample().identity, Evidence::Strong(_)));
    p.disabled_readers.insert(settings::Reader::MchoseHid);
    assert!(!rf.direct_reader_enabled(&p));
    assert!(wired.direct_reader_enabled(&p));
    p.enabled_experimental.clear();
    assert!(!wired.direct_reader_enabled(&p));
}
#[test]
fn brand_candidate_is_not_starved_by_history_only_metadata() {
    let mut targets: Vec<_> = (0..battery::MAX_DEVICES)
        .map(|i| {
            let mut p = system::Peripheral::test_paired(&format!("old-{i}"), false);
            p.id = format!("a-{i}");
            p.kind = Kind::Phone;
            Target::Windows(p)
        })
        .collect();
    let mut headset = system::Peripheral::test_classic_headset("renamed-headset");
    headset.id = "zz-headset".into();
    targets.push(Target::Windows(headset));
    let original = targets.clone();
    prioritize_targets(&mut targets, &settings::ProviderPreferences::default());
    assert!(targets.iter().any(|t| t.id() == "zz-headset"));
    let mut disabled = settings::ProviderPreferences::default();
    disabled
        .disabled_readers
        .insert(settings::Reader::SennheiserGaia);
    let mut history = original;
    prioritize_targets(&mut history, &disabled);
    assert!(!history.iter().any(|t| t.id() == "zz-headset"));
}
#[test]
fn sennheiser_components_survive_worker_publication() {
    let mut sample = target("headset", "sdp-route").sample();
    sample.source = Source::Sennheiser;
    sample.result = Ok(battery::Reading {
        value: battery::Value::reported(65),
        kind: Kind::Headset,
        power: battery::Power::Unknown,
        connected: true,
    });
    let mut link = system::Link::default();
    link.components = vec![battery::Component {
        id: "sennheiser-left".into(),
        role: "left",
        value: battery::Value::reported(65),
        power: battery::Power::Unknown,
        external_power: None,
        identifier: None,
        raw_power_state: None,
    }];
    transfer_link_observation(&mut sample, &link);
    assert_eq!(sample.components.len(), 1);
    let tracked = battery::Tracked::update(sample, None);
    assert_eq!(tracked.components[0].role, "left");
    assert_eq!(tracked.components[0].value, battery::Value::reported(65));
}
#[test]
fn revoked_keyboard_profile_is_rejected_at_worker_execution_boundary() {
    let mut policy = settings::ProviderPreferences::default();
    let d = device::Device {
        path: vec![],
        container: None,
        vendor_id: 0x41e4,
        product_id: 0x2004,
        product: "Renamed keyboard".into(),
        usage_page: 0xff02,
        usage: 2,
        input_bytes: 20,
        output_bytes: 20,
    };
    let target = Target::Keyboard(d);
    policy
        .enabled_experimental
        .insert(settings::Profile::MchoseSinowealthExperimental);
    assert!(target.direct_reader_enabled(&policy));
    policy.enabled_experimental.clear();
    assert!(policy.source(Source::MchoseKeyboard)); // Shared reader alone is insufficient.
    assert!(!target.direct_reader_enabled(&policy));
}
fn hidpp_interface(container: &str, product_id: u16) -> device::Device {
    device::Device {
        path: format!("synthetic-hid-{container}")
            .encode_utf16()
            .chain(Some(0))
            .collect(),
        container: Some(container.into()),
        vendor_id: 0x046d,
        product_id,
        product: "Synthetic Logitech status endpoint".into(),
        usage_page: 0xff00,
        usage: 2,
        input_bytes: 20,
        output_bytes: 20,
    }
}
#[test]
fn hidpp_sources_reuse_os_actor_capacity_and_leave_receiver_slots_closed() {
    let mut targets: Vec<_> = (0..8)
        .map(|i| {
            Target::Windows(system::Peripheral::test_paired(
                &format!("physical-{i}"),
                true,
            ))
        })
        .collect();
    let interfaces: Vec<_> = (0..8)
        .map(|i| hidpp_interface(&format!("physical-{i}"), 0xb001))
        .collect();
    attach_hidpp(&mut targets, hidpp::enumerate_from(&interfaces));
    prioritize_targets(&mut targets, &settings::ProviderPreferences::default());
    assert_eq!(targets.len(), 8);
    for target in &targets {
        assert!(matches!(target,Target::Hidpp(t) if t.windows.is_some()));
        assert!(target.sources().contains(&Source::LogitechHidpp));
        assert!(target.sources().contains(&Source::Windows));
        assert!(metadata_wake(target, &None));
    }
    let mut receiver = vec![Target::Windows(system::Peripheral::test_paired(
        "receiver", true,
    ))];
    let candidates = hidpp::enumerate_from(&[hidpp_interface("receiver", 0xc52b)]);
    assert_eq!(candidates.len(), 6);
    attach_hidpp(&mut receiver, candidates);
    assert_eq!(receiver.len(), 1);
    assert!(matches!(receiver[0], Target::Windows(_)));
}
#[test]
fn hidpp_hotplug_retains_route_key_but_rebuilds_replaced_collection() {
    let interface = hidpp_interface("one", 0xb001);
    let mut targets = Vec::new();
    attach_hidpp(
        &mut targets,
        hidpp::enumerate_from(std::slice::from_ref(&interface)),
    );
    assert!(targets[0].matches_interface(&["SYNTHETIC-HID-one".into()]));
    assert!(!targets[0].matches_interface(&["unrelated".into()]));
    let mut replacement = interface;
    replacement.path = "new-hid-path".encode_utf16().chain(Some(0)).collect();
    let mut next = Vec::new();
    attach_hidpp(&mut next, hidpp::enumerate_from(&[replacement]));
    assert_eq!(targets[0].id(), next[0].id());
    assert!(targets[0] != next[0]);
}
#[test]
fn metadata_changes_wake_only_the_affected_os_actor() {
    let one = Target::Windows(system::Peripheral::test_paired("one", true));
    let two = Target::Windows(system::Peripheral::test_paired("two", true));
    assert!(metadata_wake(&one, &Some(vec!["one".into()])));
    assert!(!metadata_wake(&two, &Some(vec!["one".into()])));
    assert!(!metadata_wake(&one, &Some(vec![])));
    assert!(metadata_wake(&two, &None));
    assert!(!metadata_wake(&target("one", "receiver"), &None));
}
#[test]
fn associated_hid_sources_share_windows_actor_before_candidate_budget() {
    let mut targets: Vec<_> = (0..8)
        .map(|i| {
            Target::Windows(system::Peripheral::test_paired(
                &format!("physical-{i}"),
                i < 4,
            ))
        })
        .collect();
    let hids = (0..4)
        .map(|i| {
            hid_standard::Device::test_associated(&format!("physical-{i}"), &format!("hid-{i}"))
        })
        .collect();
    attach_standard_hid(&mut targets, hids);
    prioritize_targets(&mut targets, &settings::ProviderPreferences::default());
    assert_eq!(targets.len(), 8);
    for i in 0..8 {
        let target = targets
            .iter()
            .find(|t| t.id() == battery::key("bt", &format!("physical-{i}")))
            .unwrap();
        assert_eq!(target.sources().contains(&Source::HidStandard), i < 4);
        assert!(
            matches!(target.sample().identity, Evidence::Strong(key) if key == battery::key("container", &format!("physical-{i}")))
        );
    }
    let mut ambiguous = vec![Target::Windows(system::Peripheral::test_paired(
        "one", true,
    ))];
    attach_standard_hid(
        &mut ambiguous,
        vec![
            hid_standard::Device::test_associated("one", "hid-a"),
            hid_standard::Device::test_associated("one", "hid-b"),
        ],
    );
    assert_eq!(ambiguous.len(), 1);
    assert!(!ambiguous[0].sources().contains(&Source::HidStandard));
}
#[test]
fn duplicate_transport_does_not_consume_the_last_unique_device_slot() {
    let mut usb_a = target("a", "usb-a");
    let mut usb_b = target("b", "usb-b");
    for usb in [&mut usb_a, &mut usb_b] {
        if let Target::Mchose(device) = usb {
            device.product_id = 0x4031;
        }
    }
    let last = target("unit-last", "rf-last");
    let mut targets = vec![usb_a.clone(), usb_b.clone(), target("a", "rf-a")];
    targets.extend((2..battery::MAX_DEVICES - 1).map(|i| {
        let id = format!("unit-{i:02}");
        target(&id, &id)
    }));
    targets.push(last.clone());
    assert_eq!(targets.len(), battery::MAX_DEVICES + 1);
    prioritize_targets(&mut targets, &settings::ProviderPreferences::default());
    assert_eq!(targets.len(), battery::MAX_DEVICES);
    assert_eq!(
        targets
            .iter()
            .map(Target::id)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        battery::MAX_DEVICES
    );
    assert!(targets.contains(&usb_a));
    assert!(targets.contains(&usb_b));
    assert!(targets.contains(&last));
    assert!(!targets.contains(&target("a", "rf-a")));
    assert!(
        targets[..2]
            .iter()
            .all(|t| matches!(t, Target::Mchose(d) if mchose::profiles::mouse_wired(d)))
    );
}
#[test]
fn hotplug_matches_only_the_affected_interface_and_detects_path_replacement() {
    let mouse = target("mouse", "receiver-a");
    let keyboard = target("keyboard", "receiver-b");
    let changed = vec!["RECEIVER-A".into()];
    assert!(mouse.matches_interface(&changed));
    assert!(!keyboard.matches_interface(&changed));
    assert!(!mouse.matches_interface(&[]));
    let replacement = target("mouse", "receiver-a-new-path");
    assert_eq!(replacement.id(), mouse.id());
    assert!(replacement != mouse); // Stable identity must not keep an obsolete session.

    let standard = Target::Hid(hid_standard::Device::test_route("receiver-a"));
    assert!(standard.matches_interface(&changed));
    assert!(!standard.matches_interface(&["unrelated".into()]));
}

#[test]
fn disabled_windows_readers_do_not_consume_monitoring_slots() {
    let mut targets: Vec<_> = (0..battery::MAX_DEVICES)
        .map(|i| {
            Target::Windows(system::Peripheral::test_paired(
                &format!("connected-{i}"),
                true,
            ))
        })
        .collect();
    let standard = Target::Hid(hid_standard::Device::test_route("feature-hid"));
    targets.push(standard.clone());
    let mut policy = settings::ProviderPreferences::default();
    policy.disabled_readers.extend([
        settings::Reader::WindowsMetadata,
        settings::Reader::BluetoothGatt,
        settings::Reader::SennheiserGaia,
    ]);
    policy
        .enabled_experimental
        .insert(settings::Profile::HidStandardExperimental);
    let catalog = prioritize_targets(&mut targets, &policy);
    assert_eq!(targets.len(), 1);
    assert!(targets[0] == standard);
    assert_eq!(catalog[0].id, standard.id());
    assert_eq!(catalog[0].scheduling, "active");
}
#[test]
fn topology_merge_keeps_other_readings_and_marks_only_new_devices_pending() {
    let mouse = target("mouse", "receiver-a");
    let removed = target("removed", "receiver-old");
    let added = target("added", "receiver-new");
    let mut good = mouse.sample();
    good.at = 1234;
    good.result = Ok(battery::Reading {
        value: crate::battery::Value::reported(42),
        kind: Kind::Mouse,
        power: battery::Power::Charging,
        connected: true,
    });
    let mut state = Snapshot {
        generation: 7,
        devices: vec![good, removed.sample()],
        catalog: Vec::new(),
        error: None,
    };
    state.reconcile(&[mouse.clone(), added.clone()]);
    assert_eq!(state.generation, 7);
    assert_eq!(state.devices.len(), 2);
    let preserved = state
        .devices
        .iter()
        .find(|sample| sample.id == mouse.id())
        .unwrap();
    assert_eq!(preserved.at, 1234);
    assert_eq!(preserved.result.as_ref().unwrap().percent(), Some(42));
    assert_eq!(
        preserved.result.as_ref().unwrap().power,
        battery::Power::Charging
    );
    assert!(!state.devices.iter().any(|sample| sample.id == removed.id()));
    assert!(
        state
            .devices
            .iter()
            .find(|sample| sample.id == added.id())
            .unwrap()
            .result
            .is_err()
    );
    state.reconcile(std::slice::from_ref(&mouse));
    assert_eq!(state.devices.len(), 1);
    assert_eq!(state.devices[0].id, mouse.id());
    assert_eq!(state.devices[0].at, 1234);
}
