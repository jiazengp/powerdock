//! Agent commands reuse the host's settings, policy and snapshot ownership.
use super::*;

impl App {
    pub(super) fn agent_command(&mut self, operation: agent::Operation) -> serde_json::Value {
        let audit = !operation.snapshot_only();
        let request_id = crate::diagnostics::next_request();
        let began = Instant::now();
        if audit {
            crate::diagnostics::record(
                "operation_requested",
                serde_json::json!({"request_id":request_id,"operation":operation}),
            );
        }
        let reply = self.agent_command_inner(operation);
        if audit {
            let fields = serde_json::json!({"request_id":request_id,"elapsed_ms":began.elapsed().as_millis(),"ok":reply["ok"],"error":reply.get("error")});
            if reply["ok"] == true {
                crate::diagnostics::record("operation_completed", fields);
            } else {
                crate::diagnostics::error("operation_failed", fields);
            }
        }
        reply
    }
    pub(super) fn configure_providers(&mut self, next: Settings) -> serde_json::Value {
        if let Err(error) = settings::save(&next) {
            return agent::failure("settings_save_failed", error);
        }
        let affected = self.controls.apply(next.providers.clone());
        self.config = next;
        self.invalidate_provider_devices(&affected);
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(Command::Configure);
        }
        self.select_current();
        if let Err(e) = self.update_tray(false) {
            settings::log_error(&e);
        }
        self.publish_state();
        agent::success(
            serde_json::json!({"preferences":self.config.providers,"configuration_generation":self.controls.generation(),"refresh_pending":true}),
        )
    }
    pub(super) fn invalidate_provider_devices(&mut self, affected: &[String]) {
        for d in &mut self.devices {
            if !self.config.providers.source(d.source)
                || affected.iter().any(|id| {
                    id == &d.id
                        || self
                            .endpoint_ids
                            .get(&d.id)
                            .is_some_and(|ids| ids.contains(id))
                })
            {
                d.error = Some("电量读取器已关闭".into());
                self.notice_queue.remove(&d.id);
            }
        }
    }
    pub(super) fn agent_command_inner(&mut self, operation: agent::Operation) -> serde_json::Value {
        use agent::{Operation, failure, success};
        use serde_json::json;
        if self.demo && !operation.preview_allowed() {
            return failure(
                "preview_read_only",
                "Preview never writes configuration, queries hardware or exits through Agent control",
            );
        }
        match operation {
            Operation::ProvidersGet {} => success(json!({"preferences":self.config.providers,
                "configuration_generation":self.controls.generation(),
                "readers":settings::Reader::all().iter().map(|reader| reader.id()).collect::<Vec<_>>(),
                "profiles":settings::Profile::all(),"experimental_profiles":settings::Profile::experimental_variants()})),
            Operation::ProviderReaderSet { reader, enabled } => {
                let mut next = self.config.clone();
                if enabled {
                    next.providers.disabled_readers.remove(&reader);
                } else {
                    next.providers.disabled_readers.insert(reader);
                }
                self.configure_providers(next)
            }
            Operation::ProviderProfileSet {
                profile,
                enabled,
                experimental,
            } => {
                if (enabled && profile.experimental() && !experimental)
                    || (!profile.experimental() && experimental)
                {
                    return failure(
                        "invalid_profile_opt_in",
                        "Experimental opt-in must match an implemented experimental profile",
                    );
                }
                let mut next = self.config.clone();
                if enabled {
                    next.providers.disabled_profiles.remove(&profile);
                    if experimental {
                        next.providers.enabled_experimental.insert(profile);
                    }
                } else {
                    next.providers.disabled_profiles.insert(profile);
                    next.providers.enabled_experimental.remove(&profile);
                }
                self.configure_providers(next)
            }
            Operation::Status {} => {
                let mut state = self.state_snapshot(false);
                state["application_version"] = json!(env!("CARGO_PKG_VERSION"));
                state["running"] = json!(true);
                success(state)
            }
            Operation::Devices { all } => success(
                json!({"devices":self.device_snapshot(all, settings::now()),"selected_id":self.selected,"refresh_pending":self.busy}),
            ),
            Operation::DevicePage {
                all,
                cursor,
                page_size,
            } => agent::devices_page(
                &self.device_snapshot(all, settings::now()),
                cursor.as_deref(),
                page_size,
                all,
                self.selected.as_deref(),
                self.busy,
            ),
            Operation::DeviceCatalog { cursor, page_size } => {
                agent::catalog_page(&self.catalog_snapshot(), cursor.as_deref(), page_size)
            }
            Operation::SettingsGet {} => success(self.settings_snapshot()),
            Operation::DeviceReminderGet { id } => {
                let id = self
                    .directory
                    .as_ref()
                    .map_or_else(|| id.clone(), |d| d.canonical(&id));
                let device = self.devices.iter().find(|d| d.id == id);
                if !settings::device_id_valid(&id)
                    || (device.is_none() && !self.config.device_thresholds.contains_key(&id))
                {
                    return failure(
                        "device_not_found",
                        "Use an id from devices list --all or an existing reminder override",
                    );
                }
                success(self.reminder_state(&id, device))
            }
            Operation::DeviceReminderSet { id, threshold } => {
                if !self.identity_ready(&id) {
                    return failure(
                        "identity_unavailable",
                        "Device identity is not committed; reminder overrides are not persisted",
                    );
                }
                let id = self
                    .directory
                    .as_ref()
                    .map_or_else(|| id.clone(), |d| d.canonical(&id));
                if !self.devices.iter().any(|d| d.id == id)
                    && !self.config.device_thresholds.contains_key(&id)
                {
                    return failure(
                        "device_not_found",
                        "Use an id from devices list --all or an existing reminder override",
                    );
                }
                let mut next = self.config.clone();
                if let Err(error) = next.configure_device_threshold(&id, threshold) {
                    return failure("invalid_device_reminder", error);
                }
                if let Err(error) = settings::save(&next) {
                    return failure("settings_save_failed", error);
                }
                self.config = next;
                self.publish_state();
                success(self.reminder_state(&id, self.devices.iter().find(|d| d.id == id)))
            }
            Operation::Refresh { id } => {
                let id = id.map(|id| {
                    self.directory
                        .as_ref()
                        .map_or_else(|| id.clone(), |d| d.canonical(&id))
                });
                if let Some(id) = id {
                    let Some(device) = self.devices.iter().find(|d| d.id == id) else {
                        return failure("device_not_found", "Use an id from devices list");
                    };
                    if device.source == Source::Detected {
                        return failure(
                            "unsupported_device",
                            "This receiver is detected but has no verified battery reader",
                        );
                    }
                    if !self.refresh_device(&id) {
                        return failure(
                            "refresh_not_accepted",
                            "A refresh was just requested or the bounded queue is busy; retry after two seconds",
                        );
                    }
                } else {
                    if self
                        .last_manual
                        .is_some_and(|at| at.elapsed() < Duration::from_secs(2))
                    {
                        return failure("refresh_not_accepted", "Retry after two seconds");
                    }
                    self.refresh(false);
                }
                success(json!({"accepted":true,"completed":false,"next":"agent devices list"}))
            }
            Operation::NotificationTest {} => {
                let accepted = self.send_notice(
                    "test",
                    &i18n::text("TestNotification"),
                    &i18n::text("TestNotificationBody"),
                    true,
                );
                if accepted {
                    success(
                        json!({"accepted":true,"completed":false,"style":self.config.notifications.style}),
                    )
                } else {
                    failure(
                        "notification_failed",
                        self.notice_error
                            .as_deref()
                            .unwrap_or("No notification channel accepted the request"),
                    )
                }
            }
            Operation::UpdateStatus {} => {
                success(serde_json::to_value(&self.updates.status).unwrap())
            }
            Operation::UpdateCheck { download } => {
                if !self.updates.request(download) {
                    return failure(
                        "update_not_accepted",
                        "An update is already running or ready, or this is a legacy portable build",
                    );
                }
                unsafe {
                    SetTimer(self.host, TIMER_UPDATE, 1000, None);
                }
                self.publish_state();
                success(json!({"accepted":true,"completed":false,"next":"agent updates status"}))
            }
            Operation::UpdateApply {} => {
                if self.updates.status.state != "ready" {
                    return failure("update_not_ready", "Download and verify an update first");
                }
                if let Err(error) = self.updates.apply(true) {
                    self.publish_state();
                    return failure("update_apply_failed", error);
                }
                unsafe {
                    PostQuitMessage(0);
                }
                success(json!({"accepted":true,"completed":false}))
            }
            Operation::GeneralSet { patch } => {
                if let Err(error) = patch.validate() {
                    return failure("invalid_arguments", error);
                }
                let mut next = self.config.clone();
                if let Some(value) = patch.language {
                    next.language = value;
                }
                if let Some(value) = patch.theme {
                    next.theme = value;
                }
                if let Some(value) = patch.reduce_motion {
                    next.reduce_motion = value;
                }
                if let Some(value) = patch.auto_check {
                    next.updates.auto_check = value;
                }
                if let Some(value) = patch.auto_download {
                    next.updates.auto_download = value;
                }
                if let Some(value) = patch.preview_updates {
                    next.updates.preview = value;
                }
                if let Some(value) = patch.notification_style {
                    next.notifications.style = value;
                }
                if let Some(value) = patch.notification_sound {
                    next.notifications.sound = value;
                }
                if let Some(value) = patch.notification_after_fullscreen {
                    next.notifications.after_fullscreen = value;
                }
                if let Some(value) = patch.overlay_position {
                    next.notifications.position = value;
                }
                if let Some(value) = patch.overlay_duration {
                    next.notifications.duration_secs = value;
                }
                if let Some(value) = patch.overlay_monitor {
                    next.notifications.monitor = value;
                }
                if let Some(value) = patch.overlay_transparency {
                    next.notifications.transparency_percent = value;
                }
                if let Err(error) = settings::save(&next) {
                    return failure("settings_save_failed", error);
                }
                if !next.notifications.style.overlay() {
                    self.notice_queue.cancel();
                }
                if !next.notifications.after_fullscreen {
                    self.notice_queue.clear_deferred();
                }
                let channel_changed = next.updates.preview != self.config.updates.preview;
                self.config = next;
                i18n::set_language(self.config.language);
                if channel_changed {
                    self.updates.reset(self.config.updates.preview);
                }
                let _ = self.update_tray(false);
                self.publish_state();
                success(json!({"applied":true}))
            }
            Operation::SettingsSet {
                threshold,
                interval_secs,
                startup,
                sort_by,
                sort_order,
            } => {
                if threshold.is_none()
                    && interval_secs.is_none()
                    && startup.is_none()
                    && sort_by.is_none()
                    && sort_order.is_none()
                {
                    return failure("invalid_arguments", "At least one setting is required");
                }
                if threshold.is_some_and(|v| !settings::ALERT_THRESHOLDS.contains(&v))
                    || interval_secs.is_some_and(|v| !settings::POLL_INTERVALS.contains(&v))
                {
                    return failure("invalid_arguments", "Invalid threshold or refresh interval");
                }
                // Persist the prospective configuration before changing the running instance.
                let mut next = self.config.clone();
                if let Some(value) = threshold {
                    next.configure_threshold(value);
                }
                if let Some(value) = interval_secs {
                    next.interval_secs = value;
                }
                if let Some(value) = sort_by {
                    next.sort_by = value;
                }
                if let Some(value) = sort_order {
                    next.sort_order = value;
                }
                if let Err(error) = settings::save(&next) {
                    return failure("settings_save_failed", error);
                }
                self.config = next;
                let startup_error = if let Some(value) = startup
                    && startup_enabled() != value
                {
                    set_startup(value).err()
                } else {
                    None
                };
                if let Some(interval) = interval_secs {
                    self.controls.set_interval(interval);
                }
                if threshold.is_some() {
                    self.maybe_alert();
                }
                let _ = self.update_tray(false);
                self.publish_state();
                if let Some(error) = startup_error {
                    return failure(
                        "startup_write_failed",
                        format!("Other settings were applied; login startup failed: {error}"),
                    );
                }
                success(self.settings_snapshot())
            }
            Operation::Show { settings, menu } => {
                if self.headless {
                    return failure(
                        "headless_preview",
                        "The isolated test host does not open windows",
                    );
                }
                self.open_ui(if menu { 2 } else { u8::from(settings) });
                success(
                    json!({"requested":true,"page":if menu{"menu"}else if settings{"settings"}else{"devices"}}),
                )
            }
            Operation::Quit {} => {
                unsafe {
                    PostQuitMessage(0);
                }
                success(json!({"accepted":true,"completed":false}))
            }
        }
    }
}
