//! Local structured diagnostics. No stdout logging and no automatic uploads.
mod bundle;
mod privacy;
use crate::settings;
pub use bundle::export;
pub use privacy::{sanitize, sanitized_value, snapshot};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tracing_appender::{
    non_blocking::{ErrorCounter, NonBlockingBuilder, WorkerGuard},
    rolling::{RollingFileAppender, Rotation},
};

const MAX_FILE: u64 = 1024 * 1024;
const MAX_BUNDLE: usize = 8 * 1024 * 1024;
const MAX_RECENT_LOG: usize = 6;
const MAX_RECENT_DETAILS: usize = 240;
static SESSION: OnceLock<String> = OnceLock::new();
static DROPPED: OnceLock<ErrorCounter> = OnceLock::new();
static INIT_ERROR: OnceLock<String> = OnceLock::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);
static REQUEST: AtomicU64 = AtomicU64::new(1);
static READERS: OnceLock<Mutex<Vec<std::thread::JoinHandle<()>>>> = OnceLock::new();
pub fn finish_children() {
    if let Ok(mut readers) = READERS.get_or_init(Default::default).lock() {
        for reader in readers.drain(..) {
            let _ = reader.join();
        }
    }
}

pub fn root() -> PathBuf {
    std::env::var_os("POWERDOCK_DIAGNOSTICS_ROOT")
        .map_or_else(|| settings::root().join("logs"), PathBuf::from)
}
pub fn session() -> &'static str {
    SESSION.get_or_init(|| {
        format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        )
    })
}
pub fn initialize(args: &[String]) -> Option<WorkerGuard> {
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--preview" | "--version" | "--export-icons"))
        || args.get(1).is_some_and(|arg| arg == "agent")
    {
        return None;
    }
    let directory = args
        .iter()
        .position(|arg| arg == "--diagnostics-smoke")
        .and_then(|i| args.get(i + 1))
        .map_or_else(root, PathBuf::from);
    let role = if args.iter().any(|arg| arg == "--update-worker") {
        "update_worker"
    } else {
        "tray"
    };
    match initialize_at(&directory, role) {
        Ok(guard) => Some(guard),
        Err(error) => {
            let _ = INIT_ERROR.set(error);
            None
        }
    }
}
fn appender(directory: &Path, prefix: &str) -> Result<RollingFileAppender, String> {
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(prefix)
        .filename_suffix("jsonl")
        .max_log_files(7)
        .build(directory)
        .map_err(|error| error.to_string())
}
fn initialize_at(directory: &Path, role: &'static str) -> Result<WorkerGuard, String> {
    let prefix = if role == "update_worker" {
        "powerdock-worker"
    } else {
        "powerdock"
    };
    let (writer, guard) = NonBlockingBuilder::default()
        .buffered_lines_limit(1024)
        .lossy(true)
        .thread_name("powerdock-log")
        .finish(appender(directory, prefix)?);
    let _ = DROPPED.set(writer.error_counter());
    let level =
        if std::env::var("POWERDOCK_LOG").is_ok_and(|value| value.eq_ignore_ascii_case("debug")) {
            tracing::Level::DEBUG
        } else {
            tracing::Level::INFO
        };
    tracing_subscriber::fmt()
        .json()
        .with_ansi(false)
        .with_target(false)
        .with_thread_ids(true)
        .with_max_level(level)
        .with_writer(writer)
        .try_init()
        .map_err(|error| error.to_string())?;
    ACTIVE.store(true, Ordering::Relaxed);
    let panic_directory = directory.to_owned();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        // Release builds abort without dropping WorkerGuard; persist the panic synchronously.
        let report = json!({"timestamp_unix":settings::now(),"session":session(),"pid":std::process::id(),"version":env!("CARGO_PKG_VERSION"),"event":"rust_panic","error":sanitize(&panic.to_string()),"backtrace":sanitize(&std::backtrace::Backtrace::force_capture().to_string())});
        if let Ok(mut file) = appender(&panic_directory, "panic") {
            let _ = writeln!(file, "{report}");
            let _ = file.flush();
        }
        error("rust_panic", report);
        previous(panic);
    }));
    record(
        "application_started",
        json!({"version":env!("CARGO_PKG_VERSION"),"role":role,"architecture":std::env::consts::ARCH,"debug_logging":level==tracing::Level::DEBUG}),
    );
    Ok(guard)
}
pub fn record(event: &str, details: Value) {
    tracing::info!(session=session(),pid=std::process::id(),event,details=%sanitized_value(details));
}
pub fn error(event: &str, details: Value) {
    tracing::error!(session=session(),pid=std::process::id(),event,details=%sanitized_value(details));
}
pub fn debug(event: &str, details: Value) {
    tracing::debug!(session=session(),pid=std::process::id(),event,details=%sanitized_value(details));
}
pub fn next_request() -> u64 {
    REQUEST.fetch_add(1, Ordering::Relaxed)
}
pub fn status() -> Value {
    json!({"directory":root(),"active":ACTIVE.load(Ordering::Relaxed),"session":session(),"retained_daily_files":7,"queue_capacity":1024,"dropped_lines":DROPPED.get().map_or(0,ErrorCounter::dropped_lines),"initialization_error":INIT_ERROR.get(),"automatic_upload":false})
}
pub fn recent_for_ui(preview: bool) -> Value {
    recent_at(&root(), preview)
}
fn recent_at(directory: &Path, preview: bool) -> Value {
    if preview {
        return json!([]);
    }
    let Ok(directory) = fs::read_dir(directory) else {
        return json!([]);
    };
    let mut files = directory
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            (name.starts_with("powerdock.") || name.starts_with("panic."))
                && name.ends_with(".jsonl")
                && entry
                    .file_type()
                    .is_ok_and(|kind| kind.is_file() && !kind.is_symlink())
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|entry| {
        std::cmp::Reverse((
            entry.file_name().to_string_lossy().starts_with("panic."),
            entry.file_name(),
        ))
    });
    let Some(entry) = files.first() else {
        return json!([]);
    };
    let Ok(mut file) = File::open(entry.path()) else {
        return json!([]);
    };
    let length = file.metadata().map_or(0, |metadata| metadata.len());
    let skip = length.saturating_sub(MAX_FILE);
    if skip > 0 && file.seek(SeekFrom::Start(skip)).is_err() {
        return json!([]);
    }
    let mut bytes = Vec::new();
    if file.take(MAX_FILE).read_to_end(&mut bytes).is_err() {
        return json!([]);
    }
    if skip > 0 {
        bytes = bytes
            .splitn(2, |byte| *byte == b'\n')
            .nth(1)
            .unwrap_or_default()
            .to_vec();
    }
    let mut lines = String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect::<Vec<_>>();
    lines.reverse();
    json!(lines
        .into_iter()
        .take(MAX_RECENT_LOG)
        .map(|line| {
            let fields = line.get("fields").unwrap_or(&Value::Null);
            let details = fields
                .get("details")
                .and_then(Value::as_str)
                .map(summarize_log_details)
                .unwrap_or_default();
            json!({
                "timestamp":sanitize(line.get("timestamp").and_then(Value::as_str).unwrap_or_default()),
                "level":line.get("level").and_then(Value::as_str).unwrap_or_default(),
                "event":sanitize(fields.get("event").and_then(Value::as_str).unwrap_or_default()),
                "details":sanitize(&details),
            })
        })
        .collect::<Vec<_>>())
}
fn summarize_log_details(text: &str) -> String {
    let Ok(Value::Object(details)) = serde_json::from_str::<Value>(text) else {
        return text.chars().take(MAX_RECENT_DETAILS).collect();
    };
    const FIELDS: [&str; 19] = [
        "component",
        "child_pid",
        "page",
        "tab",
        "preference",
        "operation",
        "request_id",
        "elapsed_ms",
        "ok",
        "id",
        "source",
        "version",
        "role",
        "error",
        "message",
        "exit_code",
        "success",
        "bytes",
        "duration_ms",
    ];
    let mut values = std::collections::BTreeMap::new();
    for scope in [
        &details,
        details
            .get("data")
            .and_then(Value::as_object)
            .unwrap_or(&details),
        details
            .get("data")
            .and_then(Value::as_object)
            .and_then(|data| data.get("details"))
            .and_then(Value::as_object)
            .unwrap_or(&details),
    ] {
        for field in FIELDS {
            if let Some(value) = scope.get(field) {
                values.entry(field).or_insert(value);
            }
        }
    }
    let summary = FIELDS
        .iter()
        .filter_map(|field| {
            values
                .get(*field)
                .and_then(|value| plain_log_value(value))
                .map(|value| format!("{field}={value}"))
        })
        .collect::<Vec<_>>()
        .join(" · ");
    summary.chars().take(MAX_RECENT_DETAILS).collect()
}
fn plain_log_value(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Null => None,
        Value::Object(values) => ["operation", "name", "message"]
            .iter()
            .find_map(|key| values.get(*key).and_then(plain_log_value)),
        Value::Array(_) => None,
    }
}
pub fn prepare_child(command: &mut Command, preview: bool) {
    if !preview && ACTIVE.load(Ordering::Relaxed) {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
    }
}
pub fn attach_child(child: &mut Child, component: &'static str) {
    let pid = child.id();
    record(
        "child_started",
        json!({"component":component,"child_pid":pid}),
    );
    if let Some(stdout) = child.stdout.take() {
        capture(stdout, component, pid, false);
    }
    if let Some(stderr) = child.stderr.take() {
        capture(stderr, component, pid, true);
    }
}
fn capture(reader: impl Read + Send + 'static, component: &'static str, pid: u32, is_error: bool) {
    let spawned=std::thread::Builder::new().name(format!("log-{component}" )).spawn(move || {
        let mut reader=BufReader::new(reader);
        let mut bytes=Vec::with_capacity(4096);
        loop {
            bytes.clear();
            match reader.by_ref().take(8192).read_until(b'\n',&mut bytes) {
                Ok(0)=>break,
                Ok(_)=>{
                    let line=String::from_utf8_lossy(&bytes);let line=line.trim();if line.is_empty(){continue;}
                    let detail=line.strip_prefix("PD_TRACE ").and_then(|text|serde_json::from_str::<Value>(text).ok()).unwrap_or_else(||json!({"message":line}));
                    let fields=json!({"component":component,"child_pid":pid,"data":detail});
                    if is_error{error("native_error",fields)}else{record("native_event",fields)}
                }
                Err(failure)=>{error("child_log_read_failed",json!({"component":component,"child_pid":pid,"error":failure.to_string()}));break;}
            }
        }
    });
    if let Ok(thread) = spawned
        && let Ok(mut readers) = READERS.get_or_init(Default::default).lock()
    {
        readers.retain(|reader| !reader.is_finished());
        readers.push(thread);
    }
}
#[cfg(test)]
mod tests;
