use super::*;
use std::{
    sync::atomic::AtomicU64,
    time::{Duration, Instant},
};

pub(crate) fn interface(container: &str, pid: u16) -> device::Device {
    let interface = if pid == 0x1880 { "02" } else { "03" };
    device::Device {
        path: format!("\\\\?\\hid#vid_1038&pid_{pid:04x}&mi_{interface}#synthetic-{container}\0")
            .encode_utf16()
            .collect(),
        container: Some(container.into()),
        vendor_id: 0x1038,
        product_id: pid,
        product: "Renamed device".into(),
        usage_page: 0xff00,
        usage: 1,
        input_bytes: 65,
        output_bytes: 65,
    }
}

#[test]
fn exact_direct_models_and_vendor_tlc_are_required() {
    for (pid, command, wired, usb_interface) in [
        (0x1890, 0xd2, false, 3),
        (0x1892, 0x92, true, 3),
        (0x1880, 0x92, false, 2),
    ] {
        let d = interface("unit", pid);
        let p = profile(&d).unwrap();
        assert_eq!(
            (p.command, p.wired, p.interface),
            (command, wired, usb_interface)
        );
        let mut wrong = d.clone();
        wrong.path = if pid == 0x1880 {
            "synthetic&mi_03\0"
        } else {
            "synthetic&mi_02\0"
        }
        .encode_utf16()
        .collect();
        assert!(profile(&wrong).is_none());
        assert_ne!(p.name, d.product);
        assert_eq!(enumerate_from(std::slice::from_ref(&d)).len(), 1);
    }
    for pid in [0, 0x1838, 0x1840, 0x1830, 0x1726] {
        assert!(profile(&interface("excluded-family", pid)).is_none());
    }
    for mutation in 0..8 {
        let mut d = interface("unit", 0x1890);
        match mutation {
            0 => d.vendor_id = 0x1234,
            1 => d.usage_page = 1,
            2 => d.usage = 0,
            3 => d.input_bytes = 63,
            4 => d.output_bytes = 66,
            5 => {
                d.path = "synthetic&mi_030\0".encode_utf16().collect();
            }
            6 => {
                d.path = "synthetic&col03\0".encode_utf16().collect();
            }
            _ => {
                d.path.pop();
            }
        }
        assert!(profile(&d).is_none(), "mutation {mutation}");
    }
}

#[test]
fn ambiguous_collections_are_rejected_and_distinct_devices_remain_separate() {
    let a = interface("one", 0x1890);
    let mut duplicate = a.clone();
    duplicate.path = "synthetic&mi_03&col02\0".encode_utf16().collect();
    assert!(enumerate_from(&[a.clone(), duplicate]).is_empty());
    let b = interface("two", 0x1890);
    assert_eq!(enumerate_from(&[a.clone(), b.clone()]).len(), 2);
    assert_ne!(id(&a), id(&b));
}

#[test]
fn only_the_fixed_battery_get_and_declared_windows_shape_are_emitted() {
    for pid in [0x1890, 0x1892, 0x1880] {
        let p = model(pid).unwrap();
        for length in [64, 65] {
            let frame = request(p, length).unwrap();
            assert_eq!(frame.len(), length);
            assert_eq!(&frame[..2], &[0, p.command]);
            assert!(frame[2..].iter().all(|b| *b == 0));
        }
        for length in [0, 2, 63, 66, 520] {
            assert!(request(p, length).is_err());
        }
    }
}

#[test]
fn direct_percent_is_checked_without_guessing_power_or_coarse_scale() {
    let p = model(0x1890).unwrap();
    for length in [64, 65] {
        let mut frame = vec![0; length];
        frame[1] = p.command;
        for percent in [1, 21, 73, 100] {
            for flag in [0, 0x80] {
                frame[2] = percent | flag;
                let r = parse(p, &frame).unwrap().unwrap();
                assert_eq!(r.percent(), Some(percent));
                assert_eq!(r.power, Power::Unknown);
                assert!(!r.discharging());
            }
        }
        for raw in [0, 101, 127, 128, 255] {
            frame[2] = raw;
            assert!(parse(p, &frame).is_err());
        }
    }
}

#[test]
fn unrelated_reports_and_ambiguous_layouts_cannot_become_battery_values() {
    let p = model(0x1890).unwrap();
    let mut frame = [0; 65];
    frame[1] = 0x92;
    frame[3] = 80;
    assert!(parse(p, &frame).unwrap().is_none());
    frame[1] = p.command;
    frame[0] = 1;
    assert!(parse(p, &frame).unwrap().is_none());
    frame[0] = 0;
    frame[1] = 0x92;
    frame[2] = 80;
    assert!(parse(model(0x1880).unwrap(), &frame).is_err());
    for n in [0, 1, 3, 63] {
        assert!(parse(p, &frame[..n]).is_err());
    }
}

#[test]
fn cancellation_and_expiry_precede_any_native_open_or_submission() {
    let d = interface("never-open", 0x1890);
    let epoch = AtomicU64::new(2);
    assert!(
        query(&d, &epoch, 1, Instant::now() + Duration::from_secs(2))
            .unwrap_err()
            .starts_with("cancelled:")
    );
    assert!(
        query(&d, &epoch, 2, Instant::now() - Duration::from_millis(1))
            .unwrap_err()
            .starts_with("timeout:")
    );
}

#[test]
fn experimental_opt_in_and_reader_disable_are_persisted_and_independent() {
    use crate::settings::{Profile, ProviderPreferences, Reader};
    let mut policy = ProviderPreferences::default();
    assert!(!policy.source(battery::Source::SteelSeries));
    policy
        .enabled_experimental
        .insert(Profile::SteelseriesDirectExperimental);
    assert!(policy.source(battery::Source::SteelSeries));
    let restored: ProviderPreferences =
        serde_json::from_str(&serde_json::to_string(&policy).unwrap()).unwrap();
    assert_eq!(restored, policy);
    policy.disabled_readers.insert(Reader::SteelseriesHid);
    assert!(!policy.source(battery::Source::SteelSeries));
    assert!(policy.source(battery::Source::Mchose));
    assert_eq!(
        Reader::for_source(battery::Source::SteelSeries),
        Some(Reader::SteelseriesHid)
    );
    assert_eq!(Reader::SteelseriesHid.id(), "steelseries_hid");
}

#[test]
fn pinned_upstream_direct_replies_use_model_specific_offsets() {
    // SteelMouse d6a849bb src/protocol/hid.rs::test_decode_direct_aerox.
    // Prefix its payloads with Windows' report-ID-zero byte and pad to the descriptor size.
    for (pid, prefix, expected) in [
        (0x1890, &[0, 0xd2, 72][..], 72),
        (0x1892, &[0, 0x92, 21][..], 21),
        (0x1880, &[0, 0x92, 0, 4, 1, 100][..], 4),
    ] {
        let mut frame = [0; 65];
        frame[..prefix.len()].copy_from_slice(prefix);
        let reading = parse(model(pid).unwrap(), &frame).unwrap().unwrap();
        assert_eq!(reading.percent(), Some(expected));
        assert_eq!(reading.power, Power::Unknown);
    }
}
