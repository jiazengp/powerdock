//! Out-of-process update check/download entry point.
use super::*;

fn worker(download: bool, preview: bool, follow_system_proxy: bool) -> Reply {
    let result = (|| {
        let manager = manager(preview, follow_system_proxy)?;
        check_and_download(
            &manager,
            download,
            preview,
            &packages_dir()?,
            progress_path(std::process::id()),
        )
    })();
    worker_reply(result)
}

pub(super) fn check_and_download(
    manager: &UpdateManager,
    download: bool,
    preview: bool,
    packages: &std::path::Path,
    progress_file: PathBuf,
) -> Result<Reply, velopack::Error> {
    let mut status = Status::new("current", true);
    status.checked_at = Some(settings::now());
    match manager.check_for_updates()? {
        UpdateCheck::RemoteIsEmpty => status.state = "no_feed".into(),
        UpdateCheck::NoUpdateAvailable => {
            if !preview && !manager.get_current_version().pre.is_empty() {
                status.state = "waiting_stable".into();
            }
        }
        UpdateCheck::UpdateAvailable(info) => {
            let mut info = *info;
            status.state = "available".into();
            status.version = Some(info.TargetFullRelease.Version.clone());
            status.notes = info
                .TargetFullRelease
                .NotesMarkdown
                .chars()
                .take(1500)
                .collect();
            shorten_info(&mut info);
            let mut verified = None;
            if download {
                let target = cache_path(packages, &info.TargetFullRelease)
                    .map_err(velopack::Error::Other)?;
                // The SDK download fast path can skip an existing target file.
                // Re-download/reconstruct it through the verified path instead.
                if target.exists() {
                    fs::remove_file(&target)?;
                }

                let path = progress_file;
                if let Some(parent) = path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = fs::write(&path, "-1");
                let (tx, rx) = std::sync::mpsc::channel::<i16>();
                let progress = thread::spawn(move || {
                    let mut last = Instant::now() - Duration::from_secs(1);
                    for value in rx {
                        if last.elapsed() >= Duration::from_millis(500) || value == 100 {
                            let _ = fs::write(&path, value.to_string());
                            last = Instant::now();
                        }
                    }
                });
                let result = manager.download_updates(&info, Some(tx));
                let _ = progress.join();
                result?;
                verified = Some(digest(&target).map_err(velopack::Error::Other)?);
                status.state = "ready".into();
                status.progress = Some(100);
            }
            return Ok(Reply {
                status,
                info: Some(info),
                digest: verified,
            });
        }
    }
    Ok(Reply {
        status,
        info: None,
        digest: None,
    })
}

pub(super) fn worker_reply(result: Result<Reply, velopack::Error>) -> Reply {
    match result {
        Ok(reply) => reply,
        Err(error) => {
            let mut status = Status::new(
                if matches!(error, velopack::Error::NotInstalled(_)) {
                    "legacy"
                } else {
                    "error"
                },
                !matches!(error, velopack::Error::NotInstalled(_)),
            );
            status.details = error.to_string().chars().take(800).collect();
            if matches!(&error, velopack::Error::Io(io) if io.kind() == std::io::ErrorKind::TimedOut || windows_http::is_timeout(io))
                || matches!(&error, velopack::Error::Network(network) if matches!(network.as_ref(), velopack::NetworkError::Http(http) if http.to_string().starts_with("timeout: ")))
            {
                status.state = "timeout".into();
            } else if matches!(&error, velopack::Error::Io(io) if windows_http::is_connection_error(io))
            {
                status.state = "windows_connection_error".into();
            } else if matches!(source::http_status(&error), Some(403 | 404)) {
                status.state = "source_unavailable".into();
            }
            status.checked_at = Some(settings::now());
            Reply {
                status,
                info: None,
                digest: None,
            }
        }
    }
}
pub fn worker_entry(args: &[String]) -> i32 {
    let (action, channel, follow_system_proxy) = match args {
        [action, channel] => (action, channel, true),
        [action, channel, proxy] if proxy == "system-proxy" => (action, channel, true),
        [action, channel, proxy] if proxy == "environment-proxy" => (action, channel, false),
        _ => return 2,
    };
    if !["check", "download"].contains(&action.as_str())
        || !["stable", "preview"].contains(&channel.as_str())
    {
        return 2;
    }
    let reply = worker(
        action == "download",
        channel == "preview",
        follow_system_proxy,
    );
    match serde_json::to_string(&reply) {
        Ok(json) => {
            println!("{json}");
            0
        }
        Err(_) => 1,
    }
}
