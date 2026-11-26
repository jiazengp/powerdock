use super::*;
mod package;

#[test]
fn package_failure_text_cannot_override_the_error_category() {
    let error = velopack::Error::SizeInvalid(PathBuf::from("timeout-404.nupkg"), 403, 1);
    assert_eq!(worker::worker_reply(Err(error)).status.state, "error");
    let timeout = velopack::Error::Io(std::io::ErrorKind::TimedOut.into());
    assert_eq!(worker::worker_reply(Err(timeout)).status.state, "timeout");
    let connection_error = std::io::Error::from_raw_os_error(
        windows_sys::Win32::Networking::WinHttp::ERROR_WINHTTP_CANNOT_CONNECT as i32,
    );
    assert_eq!(
        worker::worker_reply(Err(connection_error.into()))
            .status
            .state,
        "windows_connection_error"
    );
}
#[test]
fn switching_channel_discards_a_completed_old_result() {
    let mut prefs = UpdatePreferences::default();
    let mut controller = Controller::new(true, &prefs);
    *controller.reply.lock().unwrap() = Some(Reply {
        status: Status::new("ready", true),
        info: Some(UpdateInfo::default()),
        digest: Some("old".into()),
    });
    prefs.preview = true;
    controller.configure(&prefs);
    assert!(controller.reply.lock().unwrap().is_none());
    assert!(controller.ready.is_none());
    assert!(controller.ready_digest.is_none());
}

#[test]
fn switching_proxy_discards_stale_results_and_keeps_verified_updates() {
    let mut prefs = UpdatePreferences::default();
    let mut controller = Controller::new(true, &prefs);
    let ready_reply = || Reply {
        status: Status::new("ready", true),
        info: Some(UpdateInfo::default()),
        digest: Some("verified".into()),
    };
    *controller.reply.lock().unwrap() = Some(ready_reply());
    prefs.follow_system_proxy = false;
    controller.tick(&prefs);
    assert!(controller.ready.is_none());

    *controller.reply.lock().unwrap() = Some(ready_reply());
    controller.tick(&prefs);
    prefs.follow_system_proxy = true;
    controller.configure(&prefs);
    assert!(controller.ready.is_some());
    assert_eq!(controller.ready_digest.as_deref(), Some("verified"));
}

#[test]
fn delta_cap_and_worker_argument_boundary() {
    let mut info = UpdateInfo {
        DeltasToTarget: vec![Default::default(); 6],
        ..Default::default()
    };
    shorten_info(&mut info);
    assert!(info.DeltasToTarget.is_empty());
    assert!(info.BaseRelease.is_none());
    assert_eq!(
        worker_entry(&["check".into(), "https://other-host".into()]),
        2
    );
    assert_eq!(worker_entry(&[]), 2);
    assert_eq!(
        worker_entry(&["check".into(), "preview".into(), "unknown-proxy".into()]),
        2
    );
}
