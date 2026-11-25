//! Out-of-process update check/download entry point.
use super::*;

fn worker(download: bool, preview: bool) -> Reply {
    let result = (|| {
        let manager = manager(preview)?;
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
            if matches!(&error, velopack::Error::Io(io) if io.kind() == std::io::ErrorKind::TimedOut)
                || matches!(&error, velopack::Error::Network(network) if matches!(network.as_ref(), velopack::NetworkError::Http(http) if http.to_string().starts_with("timeout: ")))
            {
                status.state = "timeout".into();
            }
            if matches!(source::http_status(&error), Some(403 | 404)) {
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
    if args.len() != 2
        || !["check", "download"].contains(&args[0].as_str())
        || !["stable", "preview"].contains(&args[1].as_str())
    {
        return 2;
    }
    let reply = worker(args[0] == "download", args[1] == "preview");
    match serde_json::to_string(&reply) {
        Ok(json) => {
            println!("{json}");
            0
        }
        Err(_) => 1,
    }
}
