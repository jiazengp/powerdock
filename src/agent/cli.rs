//! CLI parsing and routing; host-owned operations always use local IPC.
use super::ipc::call;
use super::{GeneralPatch, MAX_BYTES, Operation, failure, success};
use crate::adapters::{controllers, device, hidpp, system};
use clap::{Args, Parser, Subcommand};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    name = "powerdock agent",
    about = "Local automation CLI; structured output by default",
    version
)]
pub(super) struct Cli {
    /// Target the isolated preview host. Mutations are rejected in preview mode.
    #[arg(long, global = true)]
    preview: bool,
    /// Pretty-print JSON. Standard output always contains one JSON document.
    #[arg(long, global = true)]
    pretty: bool,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Inspect or export local diagnostic logs. Never uploads data.
    #[command(subcommand)]
    Diagnostics(DiagnosticsCommand),
    /// Discover commands, schema version, limits and data guarantees without a running host.
    Capabilities,
    /// Explain HID protocol/descriptor eligibility without battery queries or identifiers.
    Compatibility,
    /// Read the running host's snapshot; does not query hardware or start the app.
    Status,
    /// Explicit isolated controller battery probe; never merges into the monitor or stores IDs.
    ControllersProbe {
        #[arg(long, value_enum)]
        backend: controllers::Backend,
    },
    /// Enumerate metadata without querying hardware battery protocols.
    Diagnose {
        /// List paired Bluetooth endpoints/PnP metadata instead of HID interfaces.
        #[arg(long)]
        bluetooth: bool,
    },
    #[command(subcommand)]
    Devices(DeviceCommand),
    /// Inspect or configure the compiled battery-reader allowlist.
    #[command(subcommand)]
    Providers(ProviderCommand),
    #[command(subcommand)]
    Settings(SettingsCommand),
    #[command(subcommand)]
    App(AppCommand),
    #[command(subcommand)]
    Updates(UpdateCommand),
    /// Test the configured notification channels without altering alert memory.
    NotificationTest,
}
#[derive(Subcommand)]
enum DiagnosticsCommand {
    Status,
    Export {
        #[arg(long)]
        output: Option<std::path::PathBuf>,
    },
}
#[derive(Subcommand)]
enum DeviceCommand {
    /// Read connected device states from the running host.
    List {
        /// Include inactive candidates and historical metadata for diagnostics.
        #[arg(long)]
        all: bool,
        /// Return a page with at most this many devices, within the IPC byte limit.
        #[arg(long, value_parser = clap::value_parser!(u16).range(1..=32))]
        page_size: Option<u16>,
        /// Continue the same snapshot with the same filters.
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Read the discovered candidate directory, including targets deferred by the read budget.
    Catalog {
        /// Continue the same directory revision; restart without this flag after stale_revision.
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long, default_value_t = 32, value_parser = clap::value_parser!(u16).range(1..=32))]
        page_size: u16,
    },
    /// Queue a hardware refresh. Accepted does not mean a new reading is already available.
    Refresh {
        #[arg(long, value_parser=valid_id)]
        id: Option<String>,
    },
    /// Inspect or configure one device's low-battery threshold.
    #[command(subcommand)]
    Reminder(ReminderCommand),
}
#[derive(Subcommand)]
enum ReminderCommand {
    Get {
        #[arg(long, value_parser=valid_id)]
        id: String,
    },
    Set(ReminderPatch),
}
#[derive(Args)]
#[command(group(clap::ArgGroup::new("reminder_choice").required(true).args(["threshold","inherit"])))]
struct ReminderPatch {
    #[arg(long, value_parser=valid_id)]
    id: String,
    /// Override this device with a supported low-battery threshold.
    #[arg(long, value_parser=threshold)]
    threshold: Option<u8>,
    /// Remove the override and follow the global default.
    #[arg(long)]
    inherit: bool,
}
#[derive(Subcommand)]
enum SettingsCommand {
    Get,
    Set(SettingsPatch),
    /// Change language, appearance, accessibility or update preferences.
    General(GeneralPatch),
}
#[derive(Subcommand)]
enum ProviderCommand {
    List,
    Reader {
        #[arg(long, value_enum)]
        reader: crate::settings::Reader,
        #[arg(long,action=clap::ArgAction::Set, required=true)]
        enabled: bool,
    },
    Profile {
        #[arg(long, value_enum)]
        profile: crate::settings::Profile,
        #[arg(long,action=clap::ArgAction::Set, required=true)]
        enabled: bool,
        /// Explicit opt-in; only accepted for an implemented experimental profile.
        #[arg(long)]
        experimental: bool,
    },
}
#[derive(Subcommand)]
enum UpdateCommand {
    Status,
    Check,
    Download,
    Apply,
}
#[derive(Args)]
#[command(group(clap::ArgGroup::new("changes").required(true).multiple(true).args(["threshold","interval","startup","sort","order"])))]
struct SettingsPatch {
    /// Low-battery threshold from the supported presets.
    #[arg(long, value_parser=threshold)]
    threshold: Option<u8>,
    /// Poll interval in seconds from the supported presets.
    #[arg(long, value_parser=interval)]
    interval: Option<u64>,
    /// Set login startup explicitly; never toggle it implicitly.
    #[arg(long, action=clap::ArgAction::Set)]
    startup: Option<bool>,
    /// List sorting: battery, type or charging. Does not affect the tray minimum.
    #[arg(long, value_enum)]
    sort: Option<crate::settings::SortBy>,
    /// Ascending or descending; unknown values remain last.
    #[arg(long, value_enum)]
    order: Option<crate::settings::SortOrder>,
}
#[derive(Subcommand)]
enum AppCommand {
    /// Show the existing app window. Does not start a missing host.
    Show {
        #[arg(long, conflicts_with = "menu")]
        settings: bool,
        /// Show the same native menu used by the tray context action.
        #[arg(long)]
        menu: bool,
    },
    /// Ask the running host to exit normally.
    Quit,
}
fn valid_id(value: &str) -> Result<String, String> {
    if !crate::settings::device_id_valid(value) {
        Err("Use an opaque id returned by devices list (1-64 ASCII letters/digits/hyphens)".into())
    } else {
        Ok(value.into())
    }
}
fn threshold(value: &str) -> Result<u8, String> {
    value
        .parse::<u8>()
        .ok()
        .filter(|v| crate::settings::ALERT_THRESHOLDS.contains(v))
        .ok_or_else(|| {
            format!(
                "Expected one of: {}",
                crate::settings::preset_choices(&crate::settings::ALERT_THRESHOLDS)
            )
        })
}
fn interval(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .ok()
        .filter(|v| crate::settings::POLL_INTERVALS.contains(v))
        .ok_or_else(|| {
            format!(
                "Expected one of: {}",
                crate::settings::preset_choices(&crate::settings::POLL_INTERVALS)
            )
        })
}
pub fn run(args: &[String]) -> i32 {
    let parsed = Cli::try_parse_from(
        std::iter::once("powerdock agent".to_owned()).chain(args.iter().cloned()),
    );
    let cli = match parsed {
        Ok(cli) => cli,
        Err(error) if !error.use_stderr() => {
            print!("{error}");
            return 0;
        }
        Err(error) => {
            println!("{}", failure("invalid_arguments", error.to_string()));
            return 2;
        }
    };
    let operation = match cli.command {
        Command::Diagnostics(command) => {
            if cli.preview {
                emit(
                    &failure(
                        "preview_read_only",
                        "Preview does not read or export production logs",
                    ),
                    cli.pretty,
                );
                return 3;
            }
            let result = match command {
                DiagnosticsCommand::Status => {
                    let (reply, code) = call(Operation::Status {}, false);
                    Ok(if code == 0 {
                        reply["data"]["diagnostics"].clone()
                    } else {
                        crate::diagnostics::status()
                    })
                }
                DiagnosticsCommand::Export { output } => {
                    let (reply, code) = call(Operation::Status {}, false);
                    let state = if code == 0 {
                        reply["data"].clone()
                    } else {
                        Value::Null
                    };
                    crate::diagnostics::export(output.as_deref(), state)
                        .map(|path| json!({"path":path,"uploaded":false}))
                }
            };
            match result {
                Ok(data) => {
                    emit(&success(data), cli.pretty);
                    return 0;
                }
                Err(error) => {
                    emit(&failure("diagnostics_failed", error), cli.pretty);
                    return 3;
                }
            }
        }
        Command::ControllersProbe { backend } => {
            if cli.preview {
                emit(
                    &failure("preview_read_only", "Preview never queries hardware"),
                    cli.pretty,
                );
                return 3;
            }
            let _com = crate::notify::Com::initialize();
            let devices = match controllers::enumerate_experimental(backend) {
                Ok(d) => d,
                Err(e) => {
                    emit(&failure("controller_probe_failed", e), cli.pretty);
                    return 3;
                }
            };
            let generation = std::sync::atomic::AtomicU64::new(1);
            const CONTROLLER_PROBE_LIMIT: usize = 8;
            let observations:Vec<_>=devices.into_iter().take(CONTROLLER_PROBE_LIMIT).map(|d| {
                let route_only=matches!(d.identity(),crate::identity::Evidence::Route(_));
                json!({"id":d.id,"name":d.name,"kind":d.kind,"source":d.source(),"route_only":route_only,"result":d.query(&generation,1)})
            }).collect();
            emit(
                &success(
                    json!({"backend":backend,"experimental":true,"monitor_enabled":false,"reason":controllers::DISABLED_REASON,"observations":observations}),
                ),
                cli.pretty,
            );
            return 0;
        }
        Command::Compatibility if cli.preview => {
            emit(
                &failure(
                    "preview_read_only",
                    "Compatibility inventory inspects real HID metadata and is unavailable in preview.",
                ),
                cli.pretty,
            );
            return 3;
        }
        Command::Compatibility => match device::enumerate() {
            Ok(devices) => {
                let policy = call(Operation::ProvidersGet {}, false);
                emit(
                    &success(super::compatibility::inventory(&devices, policy)),
                    cli.pretty,
                );
                return 0;
            }
            Err(error) => {
                emit(&failure("diagnostic_failed", error), cli.pretty);
                return 3;
            }
        },
        Command::Capabilities => {
            emit(
                &success(
                    json!({"application_version":env!("CARGO_PKG_VERSION"),"schema_version":1,
                "transport":"same-session Windows messages; same executable path", "max_devices":crate::battery::MAX_DEVICES,
                "compatibility_inventory":{"version":2,"scope":"present_hid_interfaces","battery_protocol_queried":false,"standard_descriptors_inspected":true,"policy_source":"optional_running_host_snapshot"},
                "discovery_catalog":{"command":"devices catalog [--cursor TOKEN] [--page-size 1..32]","byte_limit":MAX_BYTES,"revision_checked":true,"deferred_reason":"budget_deferred","hardware_query":false},
                "device_pagination":{"command":"devices list [--all] --page-size 1..32 [--cursor TOKEN]","byte_limit":MAX_BYTES,"revision_checked":true,"hardware_query":false},
                "snapshot_reply_byte_limit":super::MAX_SNAPSHOT_BYTES,
                "operations":["compatibility","status","devices list [--all]","devices refresh [--id ID]","devices reminder get --id ID","devices reminder set --id ID (--threshold N | --inherit)","settings get","settings set [--threshold N] [--interval N] [--startup BOOL] [--sort battery|type|charging] [--order asc|desc]","app show [--settings|--menu]","app quit","settings general [--language system|zh-CN|en-US] [--theme system|light|dark] [--reduce-motion BOOL] [--auto-check BOOL] [--auto-download BOOL] [--preview-updates BOOL] [--follow-system-proxy BOOL]","updates status|check|download|apply","diagnose [--bluetooth]"],
                "battery_readers":crate::settings::Reader::all().iter().filter(|reader| !matches!(reader, crate::settings::Reader::Wgi | crate::settings::Reader::XInput)).map(|reader| reader.id()).collect::<Vec<_>>(),
                "snapshot_only":["status","devices list","devices catalog","settings get","devices reminder get","providers list","updates status"],"unknown_percent":null,
                "battery":{"schema_version":1,"optional_object":"battery","value_kinds":["reported_percent","ordinal","estimated_percent","unknown","no_battery"],"legacy_percent":"reported only; ordinal and estimates are null","hardware_measurement_time":"null unless provided by device","max_components_per_device":4},
                "provider_controls":{"commands":["providers list","providers reader --reader ID --enabled BOOL","providers profile --profile ID --enabled BOOL [--experimental]"],"compiled_allowlist":true,"experimental_profiles":crate::settings::Profile::experimental_variants(),"persist_before_apply":true},
                "sorting":{"default_by":"battery","default_order":"asc","by":["battery","type","charging"],"order":["asc","desc"],"unknown_last":true,"affects_tray_minimum":false},
                "refresh_is_asynchronous":true,"writes_rejected_in_preview":true,
                "notifications":{"test":"notification-test","styles":["windows","overlay","both"],"duration_presets_seconds":crate::settings::OVERLAY_DURATIONS,"transparency_percent":{"minimum":0,"maximum":100,"default":crate::settings::NotificationPreferences::default().transparency_percent},"settings_flags":["--notification-style","--notification-sound","--notification-after-fullscreen","--overlay-position","--overlay-duration","--overlay-monitor","--overlay-transparency"],"test_changes_alert_memory":false,"accepted_is_not_visibility_confirmation":true},
                "exit_codes":{"success":0,"invalid_arguments":2,"operation_failed":3,"not_running":4,"transport_failure":5},
                "diagnostics":{"status":"diagnostics status","export":"diagnostics export [--output PATH]","automatic_upload":false},"no_http_server":true,"controllers":{"mode":"diagnostic_only","probe":"controllers-probe --backend wgi|xinput","automatic_devices":controllers::enumerate().unwrap_or_default().len(),"reason":controllers::DISABLED_REASON},"hidpp":{"version":"0.9.0","verification":"protocol_only","default_enabled":false,"auto_receiver_slots":false,"receiver_reason":hidpp::RECEIVER_PAUSED},"no_mcp":true}),
                ),
                cli.pretty,
            );
            return 0;
        }
        Command::Diagnose { .. } if cli.preview => {
            emit(
                &failure(
                    "preview_read_only",
                    "Device diagnostics inspect real hardware metadata",
                ),
                cli.pretty,
            );
            return 3;
        }
        Command::Diagnose { bluetooth } => {
            let result = if bluetooth {
                system::diagnose()
            } else {
                device::enumerate().and_then(|d| serde_json::to_value(d).map_err(|e| e.to_string()))
            };
            match result {
                Ok(value) => {
                    emit(&success(value), cli.pretty);
                    return 0;
                }
                Err(error) => {
                    emit(&failure("diagnostic_failed", error), cli.pretty);
                    return 3;
                }
            }
        }
        Command::Status => Operation::Status {},
        Command::Providers(command) => match command {
            ProviderCommand::List => Operation::ProvidersGet {},
            ProviderCommand::Reader { reader, enabled } => {
                Operation::ProviderReaderSet { reader, enabled }
            }
            ProviderCommand::Profile {
                profile,
                enabled,
                experimental,
            } => Operation::ProviderProfileSet {
                profile,
                enabled,
                experimental,
            },
        },
        Command::NotificationTest => Operation::NotificationTest {},
        Command::Devices(DeviceCommand::List {
            all,
            page_size,
            cursor,
        }) => match (page_size, cursor) {
            (None, None) => Operation::Devices { all },
            (size, cursor) => Operation::DevicePage {
                all,
                cursor,
                page_size: size.unwrap_or(32),
            },
        },
        Command::Devices(DeviceCommand::Catalog { cursor, page_size }) => {
            Operation::DeviceCatalog { cursor, page_size }
        }
        Command::Devices(DeviceCommand::Refresh { id }) => Operation::Refresh { id },
        Command::Devices(DeviceCommand::Reminder(ReminderCommand::Get { id })) => {
            Operation::DeviceReminderGet { id }
        }
        Command::Devices(DeviceCommand::Reminder(ReminderCommand::Set(patch))) => {
            Operation::DeviceReminderSet {
                id: patch.id,
                threshold: if patch.inherit { None } else { patch.threshold },
            }
        }
        Command::Updates(command) => match command {
            UpdateCommand::Status => Operation::UpdateStatus {},
            UpdateCommand::Check => Operation::UpdateCheck { download: false },
            UpdateCommand::Download => Operation::UpdateCheck { download: true },
            UpdateCommand::Apply => Operation::UpdateApply {},
        },
        Command::Settings(SettingsCommand::General(patch)) => Operation::GeneralSet { patch },
        Command::Settings(SettingsCommand::Get) => Operation::SettingsGet {},
        Command::Settings(SettingsCommand::Set(patch)) => Operation::SettingsSet {
            threshold: patch.threshold,
            interval_secs: patch.interval,
            startup: patch.startup,
            sort_by: patch.sort,
            sort_order: patch.order,
        },
        Command::App(AppCommand::Show { settings, menu }) => Operation::Show { settings, menu },
        Command::App(AppCommand::Quit) => Operation::Quit {},
    };
    let (reply, code) = call(operation, cli.preview);
    emit(&reply, cli.pretty);
    code
}
fn emit(value: &Value, pretty: bool) {
    if pretty {
        println!(
            "{}",
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        );
    } else {
        println!("{value}");
    }
}
