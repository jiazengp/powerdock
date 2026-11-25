use super::*;
mod package;

#[test]
fn package_failure_text_cannot_override_the_error_category() {
    let error = velopack::Error::SizeInvalid(PathBuf::from("timeout-404.nupkg"), 403, 1);
    assert_eq!(worker::worker_reply(Err(error)).status.state, "error");
    let timeout = velopack::Error::Io(std::io::ErrorKind::TimedOut.into());
    assert_eq!(worker::worker_reply(Err(timeout)).status.state, "timeout");
}
#[test]
fn switching_channel_discards_a_completed_old_result() {
    let mut controller = Controller::new(true, false);
    *controller.reply.lock().unwrap() = Some(Reply {
        status: Status::new("ready", true),
        info: Some(UpdateInfo::default()),
        digest: Some("old".into()),
    });
    controller.reset(true);
    assert!(controller.reply.lock().unwrap().is_none());
    assert!(controller.ready.is_none());
    assert!(controller.ready_digest.is_none());
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
}
