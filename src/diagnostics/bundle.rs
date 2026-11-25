//! Bounded local export only; never uploads a diagnostic bundle.
use super::*;

pub fn export(output: Option<&Path>, state: Value) -> Result<PathBuf, String> {
    let destination = match output {
        Some(path) => path.to_owned(),
        None => {
            let folder = settings::root().join("diagnostics");
            fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
            folder.join(format!(
                "PowerDock-diagnostics-{}-{}.zip",
                session(),
                next_request()
            ))
        }
    };
    export_from(&root(), &destination, state)?;
    record(
        "diagnostics_exported",
        json!({"file":destination.file_name()}),
    );
    Ok(destination)
}
fn binaries() -> Value {
    let Ok(core) = std::env::current_exe() else {
        return Value::Null;
    };
    let ui = core
        .parent()
        .unwrap_or(Path::new("."))
        .join("ui/PowerDockUI.exe");
    let hashes = [core, ui]
        .into_iter()
        .filter_map(|path| {
            let meta = fs::metadata(&path).ok()?;
            if meta.len() > 64 * 1024 * 1024 {
                return None;
            }
            let digest = crate::updates::digest(&path).ok()?;
            Some(json!({"file":path.file_name(),"size":meta.len(),"sha256":digest}))
        })
        .collect::<Vec<_>>();
    json!(hashes)
}
fn windows_build() -> Value {
    use windows_sys::Win32::System::Registry::*;
    let key = settings::wide("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion");
    let mut result = serde_json::Map::new();
    for name in ["CurrentBuildNumber", "DisplayVersion"] {
        let mut buffer = [0u16; 128];
        let mut size = std::mem::size_of_val(&buffer) as u32;
        if unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                settings::wide(name).as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        } == 0
        {
            let count = buffer
                .iter()
                .position(|value| *value == 0)
                .unwrap_or(buffer.len());
            result.insert(
                name.into(),
                json!(String::from_utf16_lossy(&buffer[..count])),
            );
        }
    }
    Value::Object(result)
}
pub(super) fn export_from(
    directory: &Path,
    destination: &Path,
    state: Value,
) -> Result<(), String> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| error.to_string())?;
    let result = (|| {
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(1));
        let manifest = sanitized_value(
            json!({"schema_version":1,"version":env!("CARGO_PKG_VERSION"),"generated_at_unix":settings::now(),"session":session(),"architecture":std::env::consts::ARCH,"windows":windows_build(),"binaries":binaries(),"export_policy":{"max_log_bytes":MAX_BUNDLE,"max_file_tail_bytes":MAX_FILE},"diagnostics":status(),"state":snapshot(state),"privacy":{"device_names":false,"raw_hid_packets":false,"memory_dumps":false,"automatic_upload":false}}),
        );
        archive
            .start_file("manifest.json", options)
            .map_err(|error| error.to_string())?;
        archive
            .write_all(
                serde_json::to_string_pretty(&manifest)
                    .map_err(|error| error.to_string())?
                    .as_bytes(),
            )
            .map_err(|error| error.to_string())?;
        archive
            .start_file("README.txt", options)
            .map_err(|error| error.to_string())?;
        archive.write_all(b"PowerDock local diagnostics. Logs are JSON Lines ordered by timestamp. Match session, request_id and child_pid. Include the problem time, expected result and steps you performed when sharing this archive. Device names, raw packets and memory dumps are not collected. No automatic upload.\n").map_err(|error|error.to_string())?;
        let mut files = fs::read_dir(directory)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                (name.starts_with("powerdock.")
                    || name.starts_with("powerdock-worker.")
                    || name.starts_with("panic."))
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
        let mut total = 0;
        for entry in files.into_iter().take(21) {
            let mut file = File::open(entry.path()).map_err(|error| error.to_string())?;
            let length = file.metadata().map_err(|error| error.to_string())?.len();
            let skip = length.saturating_sub(MAX_FILE);
            file.seek(SeekFrom::Start(skip))
                .map_err(|error| error.to_string())?;
            let mut bytes = Vec::new();
            file.take(MAX_FILE)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            if skip > 0 {
                bytes = bytes
                    .splitn(2, |byte| *byte == b'\n')
                    .nth(1)
                    .unwrap_or_default()
                    .to_vec();
            }
            let text = sanitize(&String::from_utf8_lossy(&bytes));
            if total + text.len() > MAX_BUNDLE {
                break;
            }
            archive
                .start_file(
                    format!("logs/{}", entry.file_name().to_string_lossy()),
                    options,
                )
                .map_err(|error| error.to_string())?;
            archive
                .write_all(text.as_bytes())
                .map_err(|error| error.to_string())?;
            total += text.len();
        }
        archive.finish().map_err(|error| error.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(destination);
    }
    result
}
