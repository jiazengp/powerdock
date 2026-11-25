#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod adapters;
mod agent;
mod battery;
mod diagnostics;
mod discovery;
mod i18n;
mod icon;
mod identity;
mod monitor;
mod notify;
mod settings;
mod ui;
mod updates;
mod winrt;
use adapters::{device, mchose};
rust_i18n::i18n!("locales", fallback = "en-US");

fn main() {
    unsafe {
        use windows_sys::Win32::System::Diagnostics::Debug::*;
        SetErrorMode(GetErrorMode() | SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
    }
    let args: Vec<String> = std::env::args().collect();
    let logger = diagnostics::initialize(&args);
    let code = main_code(&args);
    diagnostics::finish_children();
    diagnostics::record("application_stopped", serde_json::json!({"exit_code":code}));
    drop(logger);
    std::process::exit(code);
}
fn main_code(args: &[String]) -> i32 {
    velopack::VelopackApp::build()
        .set_auto_apply_on_startup(false)
        .run();
    if args.iter().any(|arg| arg == "--diagnostics-smoke") {
        diagnostics::record(
            "fixture_action",
            serde_json::json!({"step":1,"action":"settings_opened"}),
        );
        diagnostics::error(
            "fixture_error",
            serde_json::json!({"step":2,"error":"Controlled diagnostic fixture"}),
        );
        if args.iter().any(|arg| arg == "--panic-fixture") {
            panic!("Controlled PowerDock panic fixture");
        }
        if args.iter().any(|arg| arg == "--native-crash-fixture") {
            let exe = std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .join("ui/PowerDockUI.exe");
            let mut command = std::process::Command::new(exe);
            command
                .args(["--smoke-crash", "--preview"])
                .env("POWERDOCK_DIAGNOSTICS_SMOKE", "1");
            diagnostics::prepare_child(&mut command, false);
            let mut child = command.spawn().expect("native fixture renderer");
            diagnostics::attach_child(&mut child, "native_crash_fixture");
            let status = child.wait().expect("wait for crash fixture");
            diagnostics::record(
                "native_crash_fixture_finished",
                serde_json::json!({"exit_code":status.code()}),
            );
            if status.success() {
                return 1;
            }
        }
        return 0;
    }
    if args.get(1).is_some_and(|arg| arg == "--update-worker") {
        return updates::worker_entry(&args[2..]);
    }
    if args.get(1).is_some_and(|arg| arg == "agent") {
        return agent::run(&args[2..]);
    }
    if let Err(error) = run() {
        diagnostics::error("application_failed", serde_json::json!({"error":error}));
        eprintln!("{error}");
        if !std::env::args().any(|arg| {
            matches!(
                arg.as_str(),
                "--probe-all"
                    | "--list-devices"
                    | "--probe"
                    | "--probe-raw"
                    | "--diagnose"
                    | "--notify-test"
                    | "--export-icons"
            )
        }) {
            ui::error_box(&error);
        }
        return 1;
    }
    0
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--version") {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if let Some(index) = args.iter().position(|arg| arg == "--export-icons") {
        let path = args
            .get(index + 1)
            .ok_or("--export-icons 需要输出 BMP 路径")?;
        return icon::export_preview(std::path::Path::new(path));
    }
    if args.iter().any(|arg| arg == "--notify-test") {
        let _com = notify::Com::initialize()?;
        notify::show(
            "PowerDock 电量提醒测试",
            "这是一条测试通知，验证系统原生电量提醒。",
            "test",
            false,
        )?;
        println!("Windows 已接受测试通知请求");
        return Ok(());
    }
    if args
        .iter()
        .any(|arg| arg == "--probe-all" || arg == "--list-devices")
    {
        let devices = if args.iter().any(|arg| arg == "--probe-all") {
            monitor::probe_all()?
        } else {
            monitor::list()?
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&devices).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--diagnose") {
        let devices = device::enumerate()?;
        println!(
            "{}",
            serde_json::to_string_pretty(&devices).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    if args
        .iter()
        .any(|arg| arg == "--probe" || arg == "--probe-raw")
    {
        let generation = std::sync::atomic::AtomicU64::new(1);
        let target = device::enumerate()?
            .into_iter()
            .filter(mchose::profiles::mchose_a7)
            .min_by_key(|d| {
                if mchose::profiles::mouse_wired(d) {
                    0
                } else {
                    1
                }
            })
            .ok_or("未找到 A7 V3 Pro 的 64 字节电量接口")?;
        let reading = if args.iter().any(|arg| arg == "--probe-raw") {
            mchose::query_diagnostic(&target, &generation, 1)?
        } else {
            mchose::query(&target, &generation, 1)?
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&reading).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    ui::run(&args)
}
