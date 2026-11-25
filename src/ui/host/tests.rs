//! Explicit desktop integration: real host dispatch, same-executable child IPC,
//! and descriptor metadata. No workers, tray, settings writes or battery GETs.
use super::*;
use serde_json::{Value, json};
use std::{path::Path, process::Stdio, time::Instant};

const OUTPUT: &str = "POWERDOCK_TEST_INVENTORY_OUTPUT";
const CLIENT_TEST: &str = "ui::host::tests::native_inventory_client";

struct Host {
    context: Box<Context>,
    window: HWND,
}
impl Host {
    fn new() -> Self {
        unsafe {
            let class = wide("PowerDock.Host");
            let title = wide("PowerDock Preview Host");
            assert!(
                FindWindowW(class.as_ptr(), title.as_ptr()).is_null(),
                "A preview is already running; do not interrupt it"
            );
            let instance = GetModuleHandleW(null());
            let registration = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..zeroed()
            };
            assert_ne!(RegisterClassW(&registration), 0);
            let mut app = super::super::observation_contract_tests::app();
            app.devices = super::super::observation_contract_tests::capacity_devices(
                &app,
                battery::MAX_DEVICES,
            );
            let context = Box::new(Context {
                generation: app.generation.clone(),
                app: UnsafeCell::new(app),
                active: Cell::new(true),
                deferred: Cell::new(0),
                pending_interfaces: RefCell::new(Vec::new()),
                host: Cell::new(null_mut()),
            });
            let window = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class.as_ptr(),
                title.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                (&*context as *const Context).cast(),
            );
            if window.is_null() {
                UnregisterClassW(class.as_ptr(), instance);
                panic!("Cannot create isolated native host");
            }
            (*context.app.get()).host = window;
            context.host.set(window);
            context.active.set(false);
            let host = Self { context, window };
            assert_eq!(FindWindowW(class.as_ptr(), title.as_ptr()), host.window);
            host
        }
    }
    fn apply(&mut self, policy: settings::ProviderPreferences) -> u64 {
        // The fixture changes only in-memory configuration between child runs.
        let app = unsafe { &mut *self.context.app.get() };
        app.controls.apply(policy.clone());
        app.config.providers = policy;
        app.controls.generation()
    }
    fn client(&self, output: &Path) {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CLIENT_TEST, "--ignored", "--nocapture"])
            .env(OUTPUT, output)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("Start same-executable IPC client");
        let began = Instant::now();
        loop {
            // SendMessageTimeout needs the actual host thread to dispatch.
            unsafe {
                let mut message: MSG = zeroed();
                while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "Native IPC child failed: {status}");
                break;
            }
            if began.elapsed() > Duration::from_secs(10) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("Native inventory roundtrip exceeded 10 seconds");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        unsafe {
            // Detach before destroy: fixture teardown must not post WM_QUIT.
            SetWindowLongPtrW(self.window, GWLP_USERDATA, 0);
            DestroyWindow(self.window);
            UnregisterClassW(wide("PowerDock.Host").as_ptr(), GetModuleHandleW(null()));
        }
    }
}

#[test]
#[ignore = "Child entry point; invoked only by native_policy_inventory_roundtrip"]
fn native_inventory_client() {
    let output = std::env::var_os(OUTPUT).expect("Only run through the parent desktop fixture");
    let inventory = agent::inventory_with_preview_host_for_test();
    assert_eq!(inventory["policy"]["source"], "running_host_snapshot");
    assert!(inventory["policy"]["error_code"].is_null());
    assert_eq!(inventory["battery_protocol_queried"], false);
    let (status, exit) = agent::call_for_test(agent::Operation::Status {}, true);
    assert_eq!(exit, 0, "{status}");
    let snapshot_bytes = status.to_string().len();
    assert!(snapshot_bytes > agent::MAX_BYTES);
    let (list, exit) = agent::call_for_test(agent::Operation::Devices { all: true }, true);
    assert_eq!(exit, 0, "{list}");
    assert_eq!(list["data"]["devices"], status["data"]["devices"]);
    assert_eq!(
        list["data"]["devices"].as_array().unwrap().len(),
        battery::MAX_DEVICES
    );
    let (legacy, exit) = agent::legacy_call_for_test(agent::Operation::Devices { all: true }, true);
    assert_eq!(exit, 3);
    assert_eq!(legacy["error"]["code"], "response_too_large");
    let result = json!({"client_pid":std::process::id(),"inventory":inventory,
        "snapshot_bytes":snapshot_bytes,"active_devices":battery::MAX_DEVICES,
        "legacy_bounded_response":true});
    std::fs::write(output, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
}

#[test]
#[ignore = "Explicit desktop/real HID metadata check; does not query battery reports"]
fn native_policy_inventory_roundtrip() {
    use settings::{Profile, ProviderPreferences, Reader};
    let mut host = Host::new();
    let output_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("validation")
        .join(format!(
            "native-policy-inventory-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    std::fs::create_dir_all(&output_root).unwrap();
    let mut disabled_readers = ProviderPreferences::default();
    disabled_readers.disabled_readers.extend(Reader::all());
    let mut enabled_experiments = ProviderPreferences::default();
    enabled_experiments
        .enabled_experimental
        .extend(Profile::experimental_variants());
    let mut disabled_profiles = enabled_experiments.clone();
    disabled_profiles.disabled_profiles.extend(Profile::all());
    let mut results = Vec::new();
    for (name, policy) in [
        ("defaults", ProviderPreferences::default()),
        ("readers_disabled", disabled_readers),
        ("experiments_enabled", enabled_experiments),
        ("profiles_disabled", disabled_profiles),
        ("restored_defaults", ProviderPreferences::default()),
    ] {
        let generation = host.apply(policy.clone());
        let output = output_root.join(format!("{name}.json"));
        host.client(&output);
        let app = unsafe { &*host.context.app.get() };
        assert!(app.worker.is_none() && app.tx.is_none());
        assert!(app.ui.is_null() && app.icon.is_null() && app.notification.is_null());
        assert!(app.child.is_none());
        let result: Value = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
        assert_ne!(result["client_pid"], std::process::id());
        let inventory = &result["inventory"];
        assert_eq!(inventory["policy"]["configuration_generation"], generation);
        let interfaces = inventory["interfaces"].as_array().unwrap();
        assert!(
            !interfaces.is_empty(),
            "This explicit check requires present HID metadata"
        );
        let mut qualified = 0;
        for assessment in interfaces
            .iter()
            .flat_map(|d| d["assessments"].as_array().unwrap())
        {
            // Descriptor failure/missing fields have no qualified reader policy.
            if assessment["configured_enabled"].is_null() {
                continue;
            }
            let reader = *Reader::all()
                .iter()
                .find(|r| assessment["reader"] == r.id())
                .unwrap();
            let profiles: Vec<Profile> =
                serde_json::from_value(assessment["profiles"].clone()).unwrap();
            let enabled = policy.reader(reader) && profiles.iter().all(|p| policy.profile(*p));
            assert_eq!(assessment["configured_enabled"], enabled);
            if assessment["stage"] == "policy" || assessment["reason"] == "eligible_for_reader" {
                let reason = if !policy.reader(reader) {
                    "reader_disabled"
                } else if profiles
                    .iter()
                    .any(|p| policy.disabled_profiles.contains(p))
                {
                    "profile_disabled"
                } else if !enabled {
                    "experimental_opt_in_required"
                } else {
                    "eligible_for_reader"
                };
                assert_eq!(assessment["reason"], reason);
            }
            qualified += 1;
        }
        assert!(
            qualified > 0,
            "A real matched reader is required for this fixture"
        );
        results.push(json!({"case":name,"generation":generation,"interfaces":interfaces.len(),
            "qualified":qualified,"client_pid":result["client_pid"],"snapshot_bytes":result["snapshot_bytes"],
            "active_devices":result["active_devices"],"legacy_bounded_response":result["legacy_bounded_response"]}));
    }
    let summary = json!({"host_pid":std::process::id(),"same_executable_cross_process_ipc":true,
        "production_host_dispatch":true,"real_hid_descriptors":true,"battery_queries":false,
        "policies_memory_only":true,"cases":results});
    std::fs::write(
        output_root.join("summary.json"),
        serde_json::to_vec_pretty(&summary).unwrap(),
    )
    .unwrap();
    println!("Native policy inventory passed: {}", output_root.display());
}
