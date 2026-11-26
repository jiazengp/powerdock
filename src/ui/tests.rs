use super::*;

#[test]
fn preview_rejects_every_mutation_without_config_changes_or_hardware_work() {
    use agent::Operation;
    let mut app = app();
    let before = serde_json::to_value(&app.config).unwrap();
    let (tx, rx) = mpsc::sync_channel(4);
    app.tx = Some(tx);
    for operation in [
        Operation::ProviderReaderSet {
            reader: settings::Reader::MchoseHid,
            enabled: false,
        },
        Operation::ProviderProfileSet {
            profile: settings::Profile::MchoseYjxExperimental,
            enabled: true,
            experimental: true,
        },
        Operation::Refresh { id: None },
        Operation::DeviceReminderSet {
            id: "demo-mouse".into(),
            threshold: Some(0),
        },
        Operation::GeneralSet {
            patch: agent::GeneralPatch::default(),
        },
        Operation::NotificationTest {},
        Operation::UpdateCheck { download: true },
        Operation::UpdateApply {},
        Operation::SettingsSet {
            threshold: Some(0),
            interval_secs: None,
            startup: None,
            sort_by: None,
            sort_order: None,
        },
        Operation::Quit {},
    ] {
        assert_eq!(
            app.agent_command_inner(operation)["error"]["code"],
            "preview_read_only"
        );
    }
    assert_eq!(serde_json::to_value(&app.config).unwrap(), before);
    assert!(rx.try_recv().is_err());
}
#[test]
fn failed_identity_migration_blocks_direct_and_alias_overrides_without_writing_settings() {
    let mut app = app();
    app.demo = false;
    let directory = app.directory.as_mut().unwrap();
    let proof = crate::identity::Evidence::Strong("proof".into());
    directory.resolve("canonical", &proof, 1).unwrap();
    directory.resolve("alias", &proof, 1).unwrap();
    app.blocked_reminders.insert("canonical".into());
    let before = serde_json::to_vec(&app.config).unwrap();
    for id in ["canonical", "alias", "session-old"] {
        assert!(!app.identity_ready(id));
        let reply = app.agent_command_inner(agent::Operation::DeviceReminderSet {
            id: id.into(),
            threshold: Some(0),
        });
        assert_eq!(reply["error"]["code"], "identity_unavailable");
    }
    assert_eq!(serde_json::to_vec(&app.config).unwrap(), before);
    app.blocked_reminders.clear();
    assert!(app.identity_ready("alias"));
    assert!(app.identity_ready("canonical"));
    assert!(!app.identity_ready("session-old"));
}
#[test]
fn identity_failures_remove_pending_and_deferred_notices_without_touching_other_devices() {
    let mut app = app();
    let old = app.devices[0].clone();
    let blocked = app.devices[1].id.clone();
    let unaffected = app.devices[2].id.clone();
    app.notice_queue.defer(&old.id);
    app.notice_queue.defer(&blocked);
    app.notice_queue.defer(&unaffected);
    for id in [&old.id, &blocked] {
        app.notice_queue.enqueue(notify::Notice::new(
            id,
            "",
            "",
            notify::TEST_ICON,
            false,
            false,
        ));
    }
    app.devices.remove(0);
    app.blocked_reminders.insert(blocked);
    app.prune_identity_notices(&[old]);
    assert_eq!(app.notice_queue.deferred_count(), 1);
    assert!(!app.notice_queue.busy());
    assert!(app.identity_ready(&unaffected));
}
#[test]
fn successful_identity_upgrade_preserves_queued_notice_and_migrated_latch() {
    let mut app = app();
    let mut old = app.devices[0].clone();
    old.id = "alias".into();
    let directory = app.directory.as_mut().unwrap();
    let proof = crate::identity::Evidence::Strong("proof".into());
    directory.resolve("canonical", &proof, 1).unwrap();
    directory.resolve("alias", &proof, 1).unwrap();
    app.config.alert_memory("alias").sent(100);
    app.config
        .merge_device_aliases("canonical", &["alias".into()]);
    let mut current = old.clone();
    current.id = "canonical".into();
    app.devices = vec![current];
    app.notice_queue.enqueue(notify::Notice::new(
        "alias",
        "low",
        "",
        notify::TEST_ICON,
        false,
        false,
    ));
    app.notice_queue.defer("alias");
    app.prune_identity_notices(&[old]);
    assert!(app.notice_queue.busy());
    assert_eq!(app.notice_queue.deferred_count(), 1);
    assert!(app.config.alert_memory("canonical").latched);
    app.notice_queue.remove("canonical");
    assert!(!app.notice_queue.busy());
    assert_eq!(app.notice_queue.deferred_count(), 0);
}
pub(super) fn app() -> App {
    let config = Settings::default();
    App {
        host: null_mut(),
        icon: null_mut(),
        icon_key: String::new(),
        guid: unsafe { zeroed() },
        taskbar_message: 0,
        notification: null_mut(),
        last: None,
        devices: battery::preview_devices(),
        catalog: Vec::new(),
        selected: None,
        error: None,
        notice_error: None,
        busy: false,
        dark: false,
        demo: true,
        window_mode: false,
        headless: true,
        generation: Arc::new(AtomicU64::new(1)),
        latest: Arc::new(Mutex::new(None)),
        tx: None,
        worker: None,
        last_manual: None,
        dpi: 96,
        child: None,
        ui: null_mut(),
        page: 0,
        updates: updates::Controller::new(true, false),
        notice_queue: notify::OverlayQueue::default(),
        directory: Some(crate::identity::Directory::default()),
        blocked_reminders: Default::default(),
        endpoint_ids: std::collections::BTreeMap::new(),
        controls: monitor::Controls::new(config.providers.clone()),
        config,
    }
}
#[test]
fn deferred_ambiguous_alias_never_hides_a_candidate_behind_a_historical_unit() {
    use crate::identity::{Directory, Evidence};
    let mut app = app();
    app.demo = false;
    app.devices.clear();
    let mut directory = Directory::default();
    directory
        .resolve(
            "direct-route",
            &Evidence::Route("shared-route-key".into()),
            1,
        )
        .unwrap();
    directory
        .resolve("direct-route", &Evidence::Strong("os-container".into()), 1)
        .unwrap();
    let unit_a = directory
        .resolve("direct-route", &Evidence::Unit("unit-a".into()), 1)
        .unwrap()
        .id;
    directory
        .resolve("old-alias", &Evidence::Strong("os-container".into()), 1)
        .unwrap();
    directory
        .resolve("direct-route", &Evidence::Unit("unit-b".into()), 2)
        .unwrap();
    assert_eq!(directory.canonical("old-alias"), unit_a);
    app.directory = Some(directory);
    app.endpoint_ids
        .insert(unit_a.clone(), vec!["current-usb".into()]);
    app.catalog = vec![
        monitor::Candidate {
            id: "current-usb".into(),
            name: "A".into(),
            kind: Kind::Mouse,
            sources: vec![Source::MchoseUsb],
            scheduling: "active",
        },
        monitor::Candidate {
            id: "old-alias".into(),
            name: "B".into(),
            kind: Kind::Mouse,
            sources: vec![Source::Mchose],
            scheduling: "budget_deferred",
        },
    ];
    let catalog = app.catalog_snapshot();
    assert_eq!(catalog.len(), 2);
    assert_eq!(catalog[0].id, unit_a);
    assert_eq!(catalog[1].id, "old-alias");
    let state = app.state_snapshot(false);
    assert_eq!(state["read_budget"]["discovered"], 2);
    assert_eq!(state["read_budget"]["budget_deferred"], 1);
}
#[test]
fn catalog_uses_canonical_ids_and_does_not_publish_deferred_battery_readings() {
    let mut app = app();
    app.demo = false;
    app.devices.clear();
    app.catalog = vec![
        monitor::Candidate {
            id: "raw-active".into(),
            name: "候选".into(),
            kind: Kind::Mouse,
            sources: vec![Source::Mchose],
            scheduling: "active",
        },
        monitor::Candidate {
            id: "raw-deferred".into(),
            name: "等待".into(),
            kind: Kind::Mouse,
            sources: vec![Source::Mchose],
            scheduling: "budget_deferred",
        },
    ];
    app.endpoint_ids.insert(
        "canonical".into(),
        vec!["raw-active".into(), "raw-deferred".into()],
    );
    let snapshot = app.catalog_snapshot();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].id, "canonical");
    assert_eq!(snapshot[0].scheduling, "active");
    let (tx, rx) = mpsc::sync_channel(4);
    app.tx = Some(tx);
    let response = app.agent_command_inner(agent::Operation::DeviceCatalog {
        cursor: None,
        page_size: 32,
    });
    assert_eq!(response["data"]["candidates"][0]["id"], "canonical");
    assert!(response["data"]["candidates"][0].get("percent").is_none());
    assert!(rx.try_recv().is_err());
    assert!(
        app.state_snapshot(true)["devices"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn unrelated_profile_preserves_readings_and_deferred_notices_but_alias_revocation_clears_them() {
    let mut app = app();
    let id = app.devices[0].id.clone();
    app.devices[0].source = Source::Mchose;
    app.devices[0].error = None;
    app.notice_queue.defer(&id);
    app.config
        .providers
        .enabled_experimental
        .insert(settings::Profile::MchoseYjxExperimental);
    app.invalidate_provider_devices(&["other-yjx".into()]);
    assert!(app.devices[0].error.is_none());
    assert_eq!(app.notice_queue.deferred_count(), 1);
    app.endpoint_ids.insert(id, vec!["raw-a7".into()]);
    app.invalidate_provider_devices(&["raw-a7".into()]);
    assert!(app.devices[0].error.is_some());
    assert_eq!(app.notice_queue.deferred_count(), 0);
}
#[test]
fn tray_withholds_the_previous_icon_while_rescanning() {
    let mut app = app();
    app.select_current();
    let now = settings::now();
    let current = app.tray_device_icon(now);
    assert_eq!((current.kind, current.fill), (Kind::Headset, Some(18)));

    // WM_RESCAN invalidates the selected reading before the worker sends a new snapshot.
    app.last = None;
    let rescanning = app.tray_device_icon(now);
    assert_eq!((rescanning.kind, rescanning.fill), (Kind::Other, None));
}

#[test]
fn old_cli_fields_preserve_precision_and_read_only_requests_enqueue_no_hardware_work() {
    let mut app = app();
    let (tx, rx) = mpsc::sync_channel(4);
    app.tx = Some(tx);
    let value = battery::Value::Ordinal {
        level: battery::Level::Low,
    };
    app.devices[0].last.as_mut().unwrap().0.value = value;
    app.select_current();
    for operation in [
        agent::Operation::Status {},
        agent::Operation::Devices { all: true },
        agent::Operation::ProvidersGet {},
        agent::Operation::SettingsGet {},
    ] {
        assert_eq!(app.agent_command_inner(operation)["ok"], true);
    }
    let state = app.state_snapshot(true);
    let coarse = state["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == app.devices[0].id)
        .unwrap();
    assert!(coarse["percent"].is_null());
    assert_eq!(coarse["available"], false);
    assert_eq!(coarse["battery"]["value_kind"], "ordinal");
    assert_eq!(coarse["battery"]["level"], "low");
    assert_eq!(coarse["battery"]["schema_version"], 1);
    for order in [settings::SortOrder::Asc, settings::SortOrder::Desc] {
        app.config.sort_order = order;
        app.state_snapshot(true);
    }
    assert!(rx.try_recv().is_err());
    app.devices[0].last.as_mut().unwrap().0.value = battery::Value::EstimatedPercent {
        percent: 1,
        model_version: 1,
    };
    let state = app.state_snapshot(true);
    let estimated = state["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == app.devices[0].id)
        .unwrap();
    assert!(estimated["percent"].is_null());
    assert_eq!(estimated["available"], false);
    assert_eq!(estimated["battery"]["value_kind"], "estimated_percent");
    app.devices[1].error = Some("temporary failure".into());
    let state = app.state_snapshot(true);
    let old = state["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == app.devices[1].id)
        .unwrap();
    assert_eq!(old["percent"], 68);
    assert_eq!(old["available"], false);
    assert_eq!(old["battery"]["freshness"], "last_known");
    assert_eq!(old["battery"]["fresh"], false);
}
#[test]
fn hidpp_snapshots_expose_the_same_reader_key_as_provider_controls() {
    let mut app = app();
    let id = app.devices[0].id.clone();
    app.devices[0].source = Source::LogitechHidpp;
    let snapshot = app.state_snapshot(true);
    let device = snapshot["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == id)
        .unwrap();
    assert_eq!(device["reader"], "logitech_hidpp");
    assert_eq!(device["battery"]["source"], "LogitechHidpp");
    let controls = app.agent_command_inner(agent::Operation::ProvidersGet {});
    assert!(
        controls["data"]["readers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| id == "logitech_hidpp")
    );
}
#[test]
fn inactive_power_preserves_legacy_enum_and_rich_state() {
    let mut app = app();
    let id = app.devices[0].id.clone();
    app.devices[0].last.as_mut().unwrap().0.power = battery::Power::Inactive;
    let snapshot = app.state_snapshot(true);
    let device = snapshot["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == id)
        .unwrap();
    assert_eq!(device["power"], "Unknown");
    assert_eq!(device["battery"]["power"], "Inactive");
}
pub(super) fn capacity_devices(app: &App, limit: usize) -> Vec<Tracked> {
    let base = app.devices[0].clone();
    (0..limit)
        .map(|i| {
            let mut d = base.clone();
            d.id = format!("device-{:057}", i);
            d.name = "键\0".repeat(24);
            d.error = Some("测\0".repeat(96));
            d.components = (0..4)
                .map(|component| battery::Component {
                    id: format!("component-{component}"),
                    role: "unknown",
                    value: battery::Value::reported(100),
                    power: battery::Power::Inactive,
                    external_power: Some(true),
                    identifier: Some(u16::MAX),
                    raw_power_state: Some(u16::MAX),
                })
                .collect();
            d
        })
        .collect()
}

#[test]
fn full_active_snapshot_and_larger_paged_snapshots_fit_their_ipc_budgets() {
    let mut app = app();
    for limit in [battery::MAX_DEVICES, 32] {
        app.devices = capacity_devices(&app, limit);
        let state = app.state_snapshot(true);
        let bytes = agent::success(state.clone()).to_string().len();
        eprintln!("{limit} devices, escaped snapshot: {bytes} bytes");
        assert!(bytes > agent::MAX_BYTES);
        if limit == battery::MAX_DEVICES {
            assert!(bytes <= agent::MAX_SNAPSHOT_BYTES);
        }
        let devices = state["devices"].as_array().unwrap();
        let mut combined = Vec::new();
        let mut cursor = None;
        loop {
            let page = agent::devices_page(devices, cursor.as_deref(), 32, true, None, false);
            assert_eq!(page["ok"], true);
            assert!(page.to_string().len() <= agent::MAX_BYTES);
            let data = &page["data"];
            assert_eq!(data["offset"], combined.len());
            assert_eq!(data["total"], limit);
            let batch = data["devices"].as_array().unwrap();
            assert!(!batch.is_empty());
            combined.extend(batch.iter().cloned());
            assert!(combined.len() <= limit);
            cursor = data["next_cursor"].as_str().map(str::to_owned);
            let Some(token) = cursor.as_deref() else {
                break;
            };
            assert_eq!(
                agent::devices_page(devices, Some(token), 32, false, None, false)["error"]["code"],
                "stale_revision"
            );
            let mut changed = devices.clone();
            changed[0]["percent"] = serde_json::json!(6);
            assert_eq!(
                agent::devices_page(&changed, Some(token), 32, true, None, false)["error"]["code"],
                "stale_revision"
            );
        }
        assert_eq!(&combined, devices);
    }
}
