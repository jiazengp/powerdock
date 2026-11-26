//! Runs in a temporary Velopack portable package created by test-updates.ps1.
use super::*;
use std::{path::Path, sync::mpsc::Sender};
use velopack::{
    Error, VelopackAsset, VelopackAssetFeed,
    bundle::Manifest,
    sources::{FileSource, UpdateSource},
};

#[derive(Clone)]
struct Recording {
    path: PathBuf,
    calls: Arc<Mutex<Vec<String>>>,
    interrupt_first_download: bool,
}
impl UpdateSource for Recording {
    fn get_release_feed(
        &self,
        channel: &str,
        app: &Manifest,
        id: &str,
    ) -> Result<VelopackAssetFeed, Error> {
        FileSource::new(&self.path).get_release_feed(channel, app, id)
    }
    fn download_release_entry(
        &self,
        asset: &VelopackAsset,
        path: &Path,
        progress: Option<Sender<i16>>,
    ) -> Result<(), Error> {
        let mut calls = self.calls.lock().unwrap();
        let interrupt = self.interrupt_first_download && calls.is_empty();
        calls.push(asset.Type.clone());
        drop(calls);
        if interrupt {
            fs::write(path, "incomplete transfer")?;
            return Err(Error::Io(std::io::ErrorKind::ConnectionReset.into()));
        }
        FileSource::new(&self.path).download_release_entry(asset, path, progress)
    }
}

#[test]
#[ignore = "Run only inside an isolated portable package from scripts/test-updates.ps1"]
fn worker_apply_restart() {
    let packages = packages_dir().unwrap();
    let app = packages.parent().unwrap();
    let root = app.parent().unwrap();
    let case = root.file_name().unwrap().to_str().unwrap();
    let target_version = if matches!(case, "delta-chain" | "missing-link") {
        "1.0.2"
    } else {
        "1.0.1"
    };
    assert!(root.join("powerdock-update-fixture").is_file());
    assert_eq!(app.file_name().unwrap(), "app");
    let locator = velopack::locator::auto_locate_app_manifest(
        velopack::locator::LocationContext::FromCurrentExe,
    )
    .unwrap();
    if locator.get_manifest_version().to_string() == target_version {
        assert_eq!(settings::root(), root.join("profile/PowerDock"));
        let (config, warning) = settings::load();
        assert!(warning.is_none(), "{warning:?}");
        assert_eq!(config.interval_secs, 120);
        assert_eq!(config.threshold_for("fixture-unit"), 10);
        assert!(!config.providers.reader(settings::Reader::MchoseHid));
        assert!(config.device_alerts["fixture-unit"].latched);
        let directory = crate::identity::Directory::load().unwrap();
        assert_eq!(directory.canonical("fixture-alias"), "fixture-unit");
        fs::write(root.join("restarted"), target_version).unwrap();
        return;
    }
    assert_eq!(locator.get_manifest_version().to_string(), "1.0.0");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let manager = UpdateManager::new(
        Recording {
            path: root.join("source"),
            calls: calls.clone(),
            interrupt_first_download: case == "interrupted",
        },
        Some(UpdateOptions {
            ExplicitChannel: Some(source::STABLE.into()),
            AllowVersionDowngrade: false,
            MaximumDeltasBeforeFallback: 5,
        }),
        None,
    )
    .unwrap();
    let progress = root.join("progress.txt");
    let check =
        worker::check_and_download(&manager, false, false, &packages, progress.clone()).unwrap();
    let check: Reply = serde_json::from_str(&serde_json::to_string(&check).unwrap()).unwrap();
    assert_eq!(check.status.state, "available");
    assert_eq!(check.status.version.as_deref(), Some(target_version));
    assert!(check.digest.is_none());
    assert!(!progress.exists());
    assert!(calls.lock().unwrap().is_empty());

    let target = cache_path(&packages, &check.info.unwrap().TargetFullRelease).unwrap();
    if case == "stale-cache" {
        fs::create_dir_all(&packages).unwrap();
        fs::write(&target, "invalid existing target").unwrap();
    }
    let download = || {
        worker::worker_reply(worker::check_and_download(
            &manager,
            true,
            false,
            &packages,
            progress.clone(),
        ))
    };
    let mut reply = download();
    if case == "interrupted" {
        assert_eq!(reply.status.state, "error");
        assert!(reply.info.is_none() && reply.digest.is_none());
        assert!(!target.exists());
        assert!(target.with_extension("partial").is_file());
        reply = download();
    }
    let reply: Reply = serde_json::from_str(&serde_json::to_string(&reply).unwrap()).unwrap();
    if case == "corrupt-full" {
        assert_eq!(reply.status.state, "error");
        assert!(reply.info.is_none() && reply.digest.is_none());
        assert!(!target.exists());
    } else {
        assert_eq!(reply.status.state, "ready", "{}", reply.status.details);
        assert_eq!(reply.status.progress, Some(100));
        assert_eq!(
            reply.digest.as_deref(),
            Some(digest(&target).unwrap().as_str())
        );
    }
    let downloads = calls.lock().unwrap().clone();
    let expected: &[&str] = match case {
        "delta" => &["Delta"],
        "delta-chain" => &["Delta", "Delta"],
        "fallback" => &["Delta", "Full"],
        "missing-link" => &["Delta", "Delta", "Full"],
        "interrupted" => &["Full", "Full"],
        "full" | "corrupt-full" | "stale-cache" | "changed-after-download" | "on-exit" => &["Full"],
        _ => panic!("Unknown update test case: {case}"),
    };
    assert_eq!(downloads, expected);

    let prefs = UpdatePreferences::default();
    let mut controller = Controller::new(true, &prefs);
    *controller.reply.lock().unwrap() = Some(reply);
    assert!(controller.tick(&prefs).is_some());
    if case == "changed-after-download" {
        fs::write(&target, "changed after download").unwrap();
        let error = controller.apply(true).unwrap_err();
        assert!(error.contains("package has changed"));
        assert!(controller.ready.is_none());
        assert_eq!(controller.status.state, "error");
    }
    fs::write(
        root.join("result.json"),
        serde_json::to_vec(&serde_json::json!({
            "case": case, "target": target_version, "downloads": downloads,
            "product_state": controller.status.state, "passed": true,
        }))
        .unwrap(),
    )
    .unwrap();
    if matches!(
        case,
        "delta" | "fallback" | "full" | "delta-chain" | "missing-link" | "interrupted"
    ) {
        controller
            .apply_verified(
                &packages,
                true,
                &[
                    "--exact",
                    "updates::tests::package::worker_apply_restart",
                    "--ignored",
                ],
            )
            .unwrap();
    } else if case == "on-exit" {
        controller.apply(false).unwrap();
        assert_eq!(controller.status.state, "applying");
    }
}
