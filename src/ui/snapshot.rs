//! Read-only projection of tracked observations and discovery metadata.
use super::*;

impl App {
    pub(super) fn settings_snapshot(&self) -> serde_json::Value {
        serde_json::json!({"threshold":self.config.threshold,"interval_secs":self.config.interval_secs,"startup":startup_enabled(),"sort_by":self.config.sort_by,"sort_order":self.config.sort_order,"device_thresholds":self.config.device_thresholds,"language":self.config.language,"theme":self.config.theme,"reduce_motion":self.config.reduce_motion,"updates":self.config.updates,"notifications":self.config.notifications,"providers":self.config.providers})
    }
    pub(super) fn reminder_state(&self, id: &str, device: Option<&Tracked>) -> serde_json::Value {
        let configured = self.config.device_thresholds.get(id).copied();
        let note = match device {
            _ if !self.identity_ready(id) => "设备目录不可用，提醒已暂停",
            Some(d) if d.source == Source::Windows => "系统缓存电量不触发提醒",
            Some(d)
                if d.reading(settings::now())
                    .is_some_and(|r| r.power == battery::Power::Unknown) =>
            {
                "充电状态未知，暂不提醒"
            }
            Some(d) if !d.visible(settings::now()) => "设备当前未确认连接",
            None => "设备当前未连接",
            _ => "",
        };
        serde_json::json!({"id":id, "threshold":configured, "inherits_global":configured.is_none(),
            "effective_threshold":self.config.threshold_for(id), "note":if !self.identity_ready(id) {i18n::text("IdentityUnavailable")} else {i18n::status(note)}})
    }
    pub(super) fn catalog_snapshot(&self) -> Vec<monitor::Candidate> {
        let mut catalog = if self.demo {
            self.devices
                .iter()
                .map(|d| monitor::Candidate {
                    id: d.id.clone(),
                    name: d.name.clone(),
                    kind: d.kind,
                    sources: vec![d.source],
                    scheduling: "active",
                })
                .collect()
        } else {
            self.catalog.clone()
        };
        for candidate in &mut catalog {
            // Historical aliases can belong to reused routes; only this batch's proven mapping may merge entries.
            candidate.id = self
                .endpoint_ids
                .iter()
                .find(|(_, ids)| ids.contains(&candidate.id))
                .map(|(id, _)| id.clone())
                .unwrap_or_else(|| candidate.id.clone());
        }
        catalog.sort_by_key(|c| (c.scheduling != "active", c.id.clone()));
        let mut seen = std::collections::HashSet::new();
        catalog.retain(|c| seen.insert(c.id.clone()));
        catalog
    }
    pub(super) fn state_snapshot(&self, include_inactive: bool) -> serde_json::Value {
        let catalog = self.catalog_snapshot();
        let age = self
            .last
            .as_ref()
            .map(|(_, at)| settings::now().saturating_sub(*at));
        let detail = if self.source() == Source::Windows {
            i18n::text("CacheDetail")
        } else if let Some(e) = &self.notice_error {
            e.clone()
        } else {
            match age {
                Some(t) if t < 60 => i18n::text("LastNow"),
                Some(t) => rust_i18n::t!("LastMinutes", minutes = t / 60).into_owned(),
                None => i18n::text("NoReading"),
            }
        };
        let percent = self
            .last
            .as_ref()
            .filter(|_| self.fresh())
            .and_then(|(r, _)| r.percent());
        let now = settings::now();
        let devices = self.device_snapshot(include_inactive, now);
        serde_json::json!({
            "version": 1,
            "device_name": self.device_name(),
            "kind": self.kind(),
            "selected": self.selected,
            "devices": devices,
            "read_budget": {"active_limit":battery::MAX_DEVICES,"max_parallel":crate::monitor::MAX_PARALLEL_QUERIES,
                "discovered":catalog.len(),"budget_deferred":catalog.iter().filter(|c|c.scheduling == "budget_deferred").count()},
            "number": self.number(),
            "percent": percent,
            "fresh": self.fresh(),
            "state": i18n::status(&self.status()).chars().take(192).collect::<String>(),
            "detail": i18n::status(&detail).chars().take(256).collect::<String>(),
            "detail_visible": self.demo || self.notice_error.is_some()
                || age.is_none_or(|seconds| seconds >= 300),
            "application_version":env!("CARGO_PKG_VERSION"),"locale":self.config.language.locale(),
            "language":self.config.language,"theme":self.config.theme,"reduce_motion":self.config.reduce_motion,
            "update_preferences":self.config.updates,"updates":self.updates.status,
            "notification_preferences":self.config.notifications,"notification_deferred":self.notice_queue.deferred_count(),
            "threshold": self.config.threshold,
            "interval_secs": self.config.interval_secs,
            "sort_by": self.config.sort_by, "sort_order": self.config.sort_order,
            "startup": startup_enabled(),
            "demo": self.demo,
            "busy": self.busy,
            "diagnostics":crate::diagnostics::status(),
            "recent_logs":crate::diagnostics::recent_for_ui(self.demo),
            "configuration_generation":self.controls.generation(),
            "notice_error": self.notice_error
        })
    }

    pub(super) fn device_snapshot(
        &self,
        include_inactive: bool,
        now: u64,
    ) -> Vec<serde_json::Value> {
        battery::ordered(&self.devices, self.config.sort_by, self.config.sort_order, now)
            .into_iter()
            .filter(|d| include_inactive || d.visible(now))
            .map(|d| {
                let available = d.reading(now).and_then(Reading::percent).is_some();
                let percent = d.last_known(now).and_then(Reading::percent);
                let description = d.error.clone().unwrap_or_else(|| {
                    if self.demo {
                        "模拟数据 · 已连接".into()
                    } else {
                        d.last.as_ref().map_or_else(
                            || "等待连接状态".into(),
                            |(r, _)| {
                                format!(
                                    "{} · {}{}",
                                    if d.source == Source::MchoseKeyboard {"接收器电量应答"}else{"已连接"},
                                    r.state(),
                                    if d.source == Source::Windows {
                                        " · 缓存"
                                    } else {
                                        ""
                                    }
                                )
                            },
                        )
                    }
                });
                serde_json::json!({
                    "id": d.id,
                    "name": d.name.chars().take(48).collect::<String>(),
                    "source": i18n::status(d.source.label()),
                    "kind": d.last.as_ref().map_or(d.kind,|(r,_)|r.kind),
                    "percent": percent,
                    "battery": d.presentation(now),
                    "available": available,
                    "power": d.reading(now).filter(|_| d.source != Source::Windows).map(|r| if r.power == battery::Power::Inactive { battery::Power::Unknown } else { r.power }),
                    "battery_tone": d.battery_tone(now),
                    "reminder": self.reminder_state(&d.id, Some(d)),
                    "connected": d.connection,
                    "observed_at": d.last.as_ref().map(|(_,at)| *at),
                    "status_observed_at": d.observed_at,
                    "measurement_at": null,
                    "source_cache": d.source == Source::Windows,
                    "error_code":d.error.as_deref().map(monitor::Failure::from_error),
                    "reader": settings::Reader::for_source(d.source).map(|reader| reader.id()).unwrap_or(if d.source == Source::Preview { "preview" } else { "none" }),
                    "state": i18n::status(&description).chars().take(192).collect::<String>()
                })
            })
            .collect()
    }
}
