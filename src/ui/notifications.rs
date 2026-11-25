//! Notification eligibility, queueing and deferred delivery.
use super::*;

impl App {
    pub(super) fn prune_identity_notices(&mut self, previous: &[Tracked]) {
        for d in previous.iter().chain(&self.devices) {
            let canonical = self
                .directory
                .as_ref()
                .map_or_else(|| d.id.clone(), |directory| directory.canonical(&d.id));
            if !self
                .devices
                .iter()
                .any(|current| current.id == canonical && self.identity_ready(&current.id))
            {
                self.notice_queue.remove(&d.id);
            } else {
                self.notice_queue.rename_device(&d.id, &canonical);
            }
        }
    }
    pub(super) fn identity_ready(&self, id: &str) -> bool {
        let canonical = self
            .directory
            .as_ref()
            .map_or_else(|| id.to_owned(), |d| d.canonical(id));
        !id.starts_with("session-") && !self.blocked_reminders.contains(&canonical)
    }
    pub(super) fn maybe_alert(&mut self) {
        if self.demo {
            return;
        }
        for device in self.devices.clone() {
            // Windows exposes cached metadata without a device measurement timestamp.
            // Do not turn an unaged OS cache into a fresh low-battery notification.
            if !self.identity_ready(&device.id)
                || self.notice_queue.awaiting_fresh_read(&device.id)
                || device.source == Source::Windows
                || device.error.is_some()
            {
                continue;
            }
            let Some(reading) = device.reading(settings::now()).cloned() else {
                continue;
            };
            let threshold = self.config.threshold_for(&device.id);
            let memory = self.config.alert_memory(&device.id);
            let before = memory.latched;
            let eligible = memory.eligible(&reading, threshold, settings::now());
            let changed = before != memory.latched;
            if eligible {
                let body = rust_i18n::t!(
                    "NotificationBody",
                    name = device.name,
                    percent = reading.percent().map(|p| p.to_string()).unwrap_or_default()
                );
                if self.send_notice(&device.id, &i18n::text("NotificationTitle"), &body, false) {
                    if let Some(memory) = self.config.device_alerts.get_mut(&device.id) {
                        memory.sent(settings::now());
                    }
                    self.save();
                }
            } else if changed {
                self.save();
            }
        }
    }
    pub(super) fn send_notice(&mut self, key: &str, title: &str, body: &str, test: bool) -> bool {
        if self.demo {
            return false;
        }
        let mut accepted = false;
        let prefs = self.config.notifications.clone();
        crate::diagnostics::record(
            "notification_requested",
            serde_json::json!({"device_id":key,"style":prefs.style,"test":test,"after_fullscreen":false,"duration_secs":prefs.duration_secs,"position":prefs.position,"transparency_percent":prefs.transparency_percent}),
        );
        if prefs.style.overlay() {
            if !test && notify::session_idle() {
                // For overlay-only alerts, record delivery after the deferred entry is replayed.
                self.notice_queue.defer_while_idle(key.into());
                self.pump_notices();
            } else {
                self.notice_queue.enqueue(notify::Notice {
                    key: key.into(),
                    title: title.into(),
                    body: body.into(),
                    sound: prefs.sound && !prefs.style.windows(),
                    test,
                });
                if prefs.after_fullscreen && !test && notify::fullscreen_application() {
                    self.notice_queue.defer(key);
                }
                self.pump_notices();
                accepted = self.notice_queue.error.is_none();
            }
        }
        if accepted {
            self.notice_error = None;
        }
        if !prefs.style.windows() {
            return accepted;
        }
        let tag = key.rsplit('-').next().unwrap_or(key);
        match notify::show(title, body, tag, !prefs.sound) {
            Ok(()) => {
                crate::diagnostics::record(
                    "windows_notification_accepted",
                    serde_json::json!({"device_id":key,"test":test,"after_fullscreen":false}),
                );
                self.notice_error = None;
                true
            }
            Err(error) => unsafe {
                settings::log_error(&format!("WinRT 通知失败，使用系统托盘提醒：{error}"));
                let mut data = self.tray_data();
                data.uFlags = NIF_GUID | NIF_INFO;
                copy_wide(&mut data.szInfoTitle, title);
                copy_wide(&mut data.szInfo, body);
                data.dwInfoFlags = NIIF_WARNING
                    | NIIF_RESPECT_QUIET_TIME
                    | if !prefs.sound { NIIF_NOSOUND } else { 0 };
                let success = Shell_NotifyIconW(NIM_MODIFY, &data) != 0;
                if !success {
                    self.notice_error = Some("系统通知发送失败，可在面板查看电量".into());
                }
                success || accepted
            },
        }
    }
    pub(super) fn pump_notices(&mut self) {
        self.notice_queue.pump(
            self.host as usize,
            self.config.reduce_motion,
            &self.config.notifications,
        );
        unsafe {
            if self.notice_queue.busy() {
                SetTimer(self.host, TIMER_NOTICE, 200, None);
            } else if self.notice_queue.waiting_for_activity() {
                SetTimer(self.host, TIMER_NOTICE, 2000, None);
            } else {
                KillTimer(self.host, TIMER_NOTICE);
            }
        }
        if let Some(error) = &self.notice_queue.error {
            self.notice_error = Some(error.clone());
        }
    }
    pub(super) fn flush_deferred_notices(&mut self) {
        if !self.config.notifications.after_fullscreen || !self.config.notifications.style.overlay()
        {
            self.notice_queue.clear_deferred();
            return;
        }
        let deferred = self.notice_queue.take_deferred();
        self.request_deferred_reads(deferred, false);
    }
    pub(super) fn on_notice_timer(&mut self) {
        if self.notice_queue.waiting_for_activity()
            && !notify::session_idle()
            && !(self.config.notifications.after_fullscreen && notify::fullscreen_application())
        {
            let deferred = self.notice_queue.take_idle_deferred();
            self.request_deferred_reads(deferred, true);
        }
        self.pump_notices();
        self.try_replay_notices();
    }
    fn request_deferred_reads(&mut self, ids: impl IntoIterator<Item = String>, after_idle: bool) {
        let ids: Vec<_> = ids
            .into_iter()
            .filter(|id| self.identity_ready(id) && self.devices.iter().any(|d| &d.id == id))
            .collect();
        if ids.is_empty() {
            return;
        }
        let endpoints = ids
            .iter()
            .flat_map(|id| {
                self.endpoint_ids
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| vec![id.clone()])
            })
            .collect();
        let requested_at = Instant::now();
        let accepted = self.tx.as_ref().is_some_and(|tx| {
            tx.try_send(Command::RefreshDevice { ids: endpoints })
                .is_ok()
        });
        for id in ids {
            if accepted {
                self.notice_queue.verifying.insert(
                    id,
                    notify::DeferredRefresh {
                        requested_at,
                        after_idle,
                    },
                );
            } else if after_idle {
                self.notice_queue.defer_while_idle(id);
            } else {
                self.notice_queue.defer(&id);
            }
        }
    }
    pub(super) fn try_replay_notices(&mut self) {
        if self.notice_queue.verifying.is_empty() || self.notice_queue.busy() {
            return;
        }
        if notify::session_idle() {
            let ids: Vec<_> = self.notice_queue.verifying.keys().cloned().collect();
            for id in ids {
                self.notice_queue.defer_while_idle(id);
            }
            self.pump_notices();
            return;
        }
        let now = settings::now();
        let ready: Vec<_> = self
            .devices
            .iter()
            .filter_map(|device| {
                let refresh = self.notice_queue.verifying.get(&device.id)?;
                notify::verified_deferred_reading(device, refresh.requested_at, now)
                    .map(|reading| (device, reading, *refresh))
            })
            .collect();
        for (device, reading, refresh) in ready {
            let id = device.id.clone();
            let threshold = self.config.threshold_for(&id);
            if !notify::eligible_deferred(device, threshold, now) {
                self.notice_queue.verifying.remove(&id);
                continue;
            }
            if self.config.notifications.after_fullscreen && notify::fullscreen_application() {
                self.notice_queue.verifying.remove(&id);
                if refresh.after_idle {
                    self.notice_queue.defer_while_idle(id);
                    self.pump_notices();
                    return;
                }
                self.notice_queue.defer(&id);
                continue;
            }
            // A prior popup may have occupied the renderer while this read was arriving.
            if device.value_clock.elapsed() > Duration::from_secs(10) {
                self.notice_queue.verifying.remove(&id);
                self.request_deferred_reads([id], refresh.after_idle);
                return;
            }
            let prefs = &self.config.notifications;
            let sound = refresh.after_idle && prefs.sound && !prefs.style.windows();
            let mark_sent = refresh.after_idle && !prefs.style.windows();
            let body = rust_i18n::t!(
                "NotificationBody",
                name = device.name,
                percent = reading.percent().map(|p| p.to_string()).unwrap_or_default()
            );
            let notice = notify::Notice {
                key: id.clone(),
                title: i18n::text("NotificationTitle"),
                body: body.into_owned(),
                sound,
                test: false,
            };
            self.notice_queue.verifying.remove(&id);
            self.notice_queue.enqueue(notice);
            crate::diagnostics::record(
                "notification_replayed",
                serde_json::json!({"device_id":id,"after_idle":refresh.after_idle}),
            );
            self.pump_notices();
            if mark_sent
                && self.notice_queue.error.is_none()
                && !self.notice_queue.awaiting_fresh_read(&id)
            {
                self.config.alert_memory(&id).sent(now);
                self.save();
            }
            break;
        }
    }
}
