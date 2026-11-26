//! Short-lived native update worker. A network or patch failure cannot stall HID or shutdown.
#[path = "updates_source.rs"]
pub mod source;
mod windows_http;
mod worker;
use crate::settings::{self, UpdatePreferences};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    os::windows::{io::AsRawHandle, process::CommandExt},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use velopack::{UpdateCheck, UpdateInfo, UpdateManager, UpdateOptions};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::JobObjects::*,
};
pub use worker::worker_entry;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Status {
    pub state: String,
    pub version: Option<String>,
    pub notes: String,
    pub details: String,
    pub progress: Option<i16>,
    pub checked_at: Option<u64>,
    pub managed: bool,
}
impl Status {
    fn new(state: &str, managed: bool) -> Self {
        Self {
            state: state.into(),
            version: None,
            notes: String::new(),
            details: String::new(),
            progress: None,
            checked_at: None,
            managed,
        }
    }
}
#[derive(Serialize, Deserialize)]
struct Reply {
    status: Status,
    info: Option<UpdateInfo>,
    #[serde(default)]
    digest: Option<String>,
}
fn cache_path(
    packages: &std::path::Path,
    asset: &velopack::VelopackAsset,
) -> Result<PathBuf, String> {
    if !source::valid_asset(asset) {
        return Err("Invalid update package".into());
    }
    Ok(packages.join(&asset.FileName))
}
fn packages_dir() -> Result<PathBuf, velopack::Error> {
    Ok(velopack::locator::auto_locate_app_manifest(
        velopack::locator::LocationContext::FromCurrentExe,
    )?
    .get_packages_dir())
}
pub(crate) fn digest(path: &std::path::Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
fn manager(preview: bool, follow_system_proxy: bool) -> Result<UpdateManager, velopack::Error> {
    UpdateManager::new(
        source::Github::new(preview, follow_system_proxy),
        Some(UpdateOptions {
            AllowVersionDowngrade: false,
            ExplicitChannel: Some(
                if preview {
                    source::PREVIEW
                } else {
                    source::STABLE
                }
                .into(),
            ),
            MaximumDeltasBeforeFallback: 5,
        }),
        None,
    )
}
pub fn launcher_path() -> PathBuf {
    let executable = std::env::current_exe().unwrap_or_default();
    if let Some(current) = executable.parent()
        && current.file_name().is_some_and(|s| s == "current")
        && let Some(root) = current.parent()
        && root.join("Update.exe").is_file()
    {
        return root.join("powerdock.exe");
    }
    executable
}
fn progress_path(pid: u32) -> PathBuf {
    settings::root()
        .join("updates")
        .join(format!("progress-{pid}.txt"))
}
fn shorten_info(info: &mut UpdateInfo) {
    info.TargetFullRelease.NotesMarkdown.clear();
    info.TargetFullRelease.NotesHtml.clear();
    if let Some(base) = &mut info.BaseRelease {
        base.NotesMarkdown.clear();
        base.NotesHtml.clear();
    }
    for delta in &mut info.DeltasToTarget {
        delta.NotesMarkdown.clear();
        delta.NotesHtml.clear();
    }
    if info.DeltasToTarget.len() > 5 {
        info.BaseRelease = None;
        info.DeltasToTarget.clear();
    }
}
struct Job(HANDLE);
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
impl Job {
    fn attach(child: &std::process::Child) -> Result<Self, String> {
        unsafe {
            let raw = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if raw.is_null() {
                return Err("Cannot create updater job".into());
            }
            let job = Self(raw);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(job.0, child.as_raw_handle().cast()) == 0
            {
                return Err("Cannot isolate updater process".into());
            }
            Ok(job)
        }
    }
}
pub struct Controller {
    pub status: Status,
    ready: Option<UpdateInfo>,
    ready_digest: Option<String>,
    reply: Arc<Mutex<Option<Reply>>>,
    stop: Arc<AtomicBool>,
    progress: Arc<Mutex<Option<i16>>>,
    task: Option<thread::JoinHandle<()>>,
    started: Instant,
    last_request: Option<Instant>,
    preview: bool,
    follow_system_proxy: bool,
    demo: bool,
}
impl Controller {
    pub fn new(demo: bool, prefs: &UpdatePreferences) -> Self {
        let managed = !demo && manager(prefs.preview, prefs.follow_system_proxy).is_ok();
        Self {
            status: Status::new(
                if demo {
                    "demo"
                } else if managed {
                    "idle"
                } else {
                    "legacy"
                },
                managed,
            ),
            ready: None,
            ready_digest: None,
            reply: Arc::new(Mutex::new(None)),
            progress: Arc::new(Mutex::new(None)),
            stop: Arc::new(AtomicBool::new(false)),
            task: None,
            started: Instant::now(),
            last_request: None,
            preview: prefs.preview,
            follow_system_proxy: prefs.follow_system_proxy,
            demo,
        }
    }
    pub fn busy(&self) -> bool {
        self.task.as_ref().is_some_and(|t| !t.is_finished())
    }
    pub fn request(&mut self, download: bool) -> bool {
        if self.demo
            || !self.status.managed
            || self.task.is_some()
            || self.ready.is_some()
            || self
                .last_request
                .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
        {
            return false;
        }
        self.cancel();
        self.status.state = if download { "downloading" } else { "checking" }.into();
        self.status.progress = None;
        self.status.details.clear();
        self.last_request = Some(Instant::now());
        let reply = self.reply.clone();
        let stop = self.stop.clone();
        let preview = self.preview;
        let follow_system_proxy = self.follow_system_proxy;
        let progress = self.progress.clone();
        if let Ok(mut p) = progress.lock() {
            *p = None;
        }
        self.task = Some(thread::spawn(move || {
            let result = (|| -> Result<Reply, String> {
                let mut child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
                    .args([
                        "--update-worker",
                        if download { "download" } else { "check" },
                        if preview { "preview" } else { "stable" },
                        if follow_system_proxy {
                            "system-proxy"
                        } else {
                            "environment-proxy"
                        },
                    ])
                    .creation_flags(0x08000000)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| e.to_string())?;
                let job = match Job::attach(&child) {
                    Ok(job) => job,
                    Err(error) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(error);
                    }
                };
                let pid = child.id();
                let path = progress_path(pid);
                let stdout = child.stdout.take().ok_or("Missing updater output")?;
                // Drain concurrently: a valid JSON result can exceed the Windows pipe buffer.
                let reader = thread::spawn(move || {
                    let mut output = String::new();
                    stdout
                        .take(32769)
                        .read_to_string(&mut output)
                        .map(|_| output)
                        .map_err(|e| e.to_string())
                });
                let deadline =
                    Instant::now() + Duration::from_secs(if download { 300 } else { 45 });
                loop {
                    if stop.load(Ordering::Acquire) || Instant::now() >= deadline {
                        drop(job);
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = reader.join();
                        let _ = fs::remove_file(&path);
                        return Err("timeout".into());
                    }
                    if let Some(exit) = child.try_wait().map_err(|e| e.to_string())? {
                        let output = reader.join().map_err(|_| "Update output reader failed")??;
                        let _ = fs::remove_file(&path);
                        if !exit.success() || output.len() > 32768 {
                            return Err("Update worker failed".into());
                        }
                        return serde_json::from_str(&output).map_err(|e| e.to_string());
                    }
                    if let Ok(value) = fs::read_to_string(&path)
                        .and_then(|v| v.trim().parse::<i16>().map_err(std::io::Error::other))
                        && (0..=100).contains(&value)
                        && let Ok(mut p) = progress.lock()
                    {
                        *p = Some(value);
                    }
                    thread::sleep(Duration::from_millis(250));
                }
            })();
            if stop.load(Ordering::Acquire) {
                return;
            }
            let result = result.unwrap_or_else(|error| {
                let mut status = Status::new(
                    if error == "timeout" {
                        "timeout"
                    } else {
                        "error"
                    },
                    true,
                );
                status.details = error;
                status.checked_at = Some(settings::now());
                Reply {
                    status,
                    info: None,
                    digest: None,
                }
            });
            if let Ok(mut slot) = reply.lock() {
                *slot = Some(result);
            }
        }));
        true
    }
    pub fn tick(&mut self, prefs: &UpdatePreferences) -> Option<u64> {
        self.configure(prefs);
        if self.busy() {
            self.status.progress = self.progress.lock().ok().and_then(|v| *v);
        }
        let reply = self.reply.lock().ok().and_then(|mut r| r.take());
        if let Some(reply) = reply {
            self.cancel();
            self.ready_digest = reply.digest;
            self.ready = if reply.status.state == "ready" {
                reply.info
            } else {
                None
            };
            self.status = reply.status;
            return self.status.checked_at;
        }
        if !self.demo
            && prefs.auto_check
            && self.status.managed
            && !self.busy()
            && self.ready.is_none()
            && self.started.elapsed() >= Duration::from_secs(60)
            && (prefs.last_checked_at > settings::now()
                || settings::now().saturating_sub(prefs.last_checked_at) >= 12 * 3600)
            && self
                .last_request
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(12 * 3600))
        {
            self.request(prefs.auto_download);
        }
        None
    }
    pub fn configure(&mut self, prefs: &UpdatePreferences) {
        let channel_changed = self.preview != prefs.preview;
        if !channel_changed && self.follow_system_proxy == prefs.follow_system_proxy {
            return;
        }
        self.preview = prefs.preview;
        self.follow_system_proxy = prefs.follow_system_proxy;
        if !channel_changed && self.ready.is_some() {
            return;
        }
        self.cancel();
        self.ready = None;
        self.ready_digest = None;
        if let Ok(mut reply) = self.reply.lock() {
            *reply = None;
        }
        if let Ok(mut progress) = self.progress.lock() {
            *progress = None;
        }
        self.last_request = None;
        self.status = Status::new(
            if self.status.managed {
                "idle"
            } else {
                "legacy"
            },
            self.status.managed,
        );
    }
    pub fn apply(&mut self, restart: bool) -> Result<(), String> {
        let result = self.prepare_apply(restart);
        self.ready = None;
        self.ready_digest = None;
        match &result {
            Ok(()) => self.status.state = "applying".into(),
            Err(error) => {
                self.status.state = "error".into();
                self.status.details = error.clone();
            }
        }
        result
    }
    fn prepare_apply(&self, restart: bool) -> Result<(), String> {
        let packages = packages_dir().map_err(|e| e.to_string())?;
        self.apply_verified(&packages, restart, &[] as &[&str])
    }
    fn apply_verified(
        &self,
        packages: &std::path::Path,
        restart: bool,
        restart_args: &[&str],
    ) -> Result<(), String> {
        let info = self.ready.as_ref().ok_or("No verified update is ready")?;
        let target = cache_path(packages, &info.TargetFullRelease)?;
        if self.ready_digest.as_deref() != Some(digest(&target)?.as_str()) {
            return Err("The verified update package has changed; download it again".into());
        }
        manager(self.preview, self.follow_system_proxy)
            .map_err(|e| e.to_string())?
            .wait_exit_then_apply_updates(&info.TargetFullRelease, true, restart, restart_args)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn cancel(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
        self.stop.store(false, Ordering::Release);
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        self.cancel();
    }
}
#[cfg(test)]
mod tests;
