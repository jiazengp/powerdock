//! Last-good observations, availability and battery receipt freshness.
use super::*;

#[derive(Clone, Debug)]
pub struct Tracked {
    pub id: String,
    pub name: String,
    pub source: Source,
    pub kind: Kind,
    pub connection: Option<bool>,
    pub observed_at: u64,
    pub last: Option<(Reading, u64)>,
    pub error: Option<String>,
    pub clock: Instant,
    pub value_clock: Instant,
    pub received_at: Option<u64>,
    pub value_changed_at: Option<u64>,
    pub components: Vec<Component>,
}
impl Tracked {
    pub fn update(sample: Sample, previous: Option<&Self>) -> Self {
        let connection = sample
            .result
            .as_ref()
            .ok()
            .map(|r| r.connected)
            .or(sample.connection);
        let kind = sample.result.as_ref().map_or(sample.kind, |r| r.kind);
        let source = if sample.result.is_err() {
            previous
                .filter(|p| p.last.is_some())
                .map_or(sample.source, |p| p.source)
        } else {
            sample.source
        };
        let (value_at, sampled_value_clock) = sample
            .value_observation
            .unwrap_or((sample.at, sample.clock));
        let successful = sample.result.as_ref().is_ok_and(Reading::linked);
        let components = if successful {
            sample.components.clone()
        } else {
            previous.map_or_else(Vec::new, |p| p.components.clone())
        };
        // Replaying a snapshot is not a new acquisition. Reading an unchanged OS cache
        // updates receipt time but does not extend the battery value's lifetime.
        let unchanged = previous.is_some_and(|p| {
            p.source == sample.source
                && p.components == sample.components
                && p.last.as_ref().is_some_and(|(r, _)| {
                    sample
                        .result
                        .as_ref()
                        .ok()
                        .is_some_and(|next| next.value == r.value)
                })
        });
        let retain_time = unchanged
            && (sample.source == Source::Windows
                || previous.is_some_and(|p| p.value_clock == sampled_value_clock));
        let value_clock = if !successful || retain_time {
            previous.map_or(sampled_value_clock, |p| p.value_clock)
        } else {
            sampled_value_clock
        };
        let (last, error) = match sample.result {
            Ok(reading) if retain_time => (
                previous.and_then(|p| p.last.as_ref().map(|(_, at)| (reading, *at))),
                None,
            ),
            Ok(reading) if reading.linked() => (Some((reading, value_at)), None),
            Ok(_) => (
                previous.and_then(|p| p.last.clone()),
                Some("连接状态未确认".into()),
            ),
            Err(error) => (previous.and_then(|p| p.last.clone()), Some(error)),
        };
        Self {
            id: sample.id,
            name: sample.name,
            source,
            components,
            observed_at: sample.at,
            kind,
            connection,
            last,
            error,
            clock: sample.clock,
            value_clock,
            received_at: if successful {
                Some(sample.at)
            } else {
                previous.and_then(|p| p.received_at)
            },
            value_changed_at: if successful && !unchanged {
                Some(value_at)
            } else {
                previous.and_then(|p| p.value_changed_at)
            },
        }
    }
    pub fn visible(&self, now: u64) -> bool {
        self.connection == Some(true) && fresh(self.observed_at, self.clock, now)
    }
    pub fn battery_tone(&self, now: u64) -> BatteryTone {
        match self.reading(now).filter(|_| self.source != Source::Windows) {
            Some(r) if matches!(r.value.normalized(), Value::Unknown | Value::NoBattery) => {
                BatteryTone::Unknown
            }
            Some(r) if r.power == Power::Charging => BatteryTone::Charging,
            Some(r) if r.percent().is_some_and(|p| p <= 20) => BatteryTone::Low,
            Some(r) if r.percent().is_some() => BatteryTone::Normal,
            Some(_) => BatteryTone::Unknown,
            None => BatteryTone::Unknown,
        }
    }
    pub fn reading(&self, now: u64) -> Option<&Reading> {
        self.last_known(now)
            .filter(|r| self.error.is_none() && self.connection == Some(true) && r.connected)
    }
    /// Legacy consumers may display a recent last-known value, but must still check
    /// available/fresh before comparing or notifying.
    pub fn last_known(&self, now: u64) -> Option<&Reading> {
        self.last
            .as_ref()
            .filter(|(_, at)| fresh(*at, self.value_clock, now))
            .map(|(reading, _)| reading)
    }
    pub fn presentation(&self, now: u64) -> serde_json::Value {
        let reading = self.last_known(now);
        let usable = self.reading(now).is_some();
        let value = reading.map_or(Value::Unknown, |r| r.value.normalized());
        let mut result = serde_json::json!({
            "schema_version":1, "display":value.display(),
            "precision": match value { Value::ReportedPercent{..} => "reported", Value::Ordinal{..} => "ordinal", Value::EstimatedPercent{..} => "estimated", _ => "unknown" },
            "presence":match value { Value::NoBattery => "absent", Value::Unknown => "unknown", _ => "present" },
            "power":reading.map(|r|r.power), "fresh":usable,
            "freshness":if usable {"fresh"} else if reading.is_some() {"last_known"} else {"unavailable"},
            "external_power":"unknown", "activity":"unknown",
            "connection":self.connection,
            "evidence_scope":if self.source==Source::MchoseKeyboard {"receiver"} else if matches!(self.source,Source::HidStandard|Source::SteelSeries) {"endpoint"} else if matches!(self.source,Source::Windows|Source::BluetoothGatt|Source::Sennheiser) {"os_profile"} else {"device"},
            "source":self.source,
            "source_cache":self.source == Source::Windows,
            "attempted_at":self.observed_at,"received_at":self.received_at,"last_success_at":self.received_at,
            "value_changed_at":self.value_changed_at,"measured_at":null,
            "components":[{"id":"main","role":"main","value":value}]
        });
        if let Some(fields) = serde_json::to_value(value)
            .ok()
            .and_then(|v| v.as_object().cloned())
        {
            result.as_object_mut().unwrap().extend(fields);
        }
        if self.components.len() == 1 {
            result["external_power"] = serde_json::json!(match self.components[0].external_power {
                Some(true) => "connected",
                Some(false) => "disconnected",
                None => "unknown",
            });
        }
        if !self.components.is_empty() {
            result["components"] = serde_json::json!(
                self.components
                    .iter()
                    .map(|c| {
                        let mut item = serde_json::to_value(c).unwrap();
                        item["observed_at"] =
                            serde_json::json!(self.last.as_ref().map(|(_, at)| *at));
                        item["fresh"] = serde_json::json!(usable);
                        item["source"] = serde_json::json!(self.source);
                        item["measured_at"] = serde_json::Value::Null;
                        item
                    })
                    .collect::<Vec<_>>()
            );
        }
        result
    }
}

pub(super) fn fresh(at: u64, clock: Instant, now: u64) -> bool {
    now.saturating_sub(at) <= FRESH_FOR.as_secs() && clock.elapsed() <= FRESH_FOR
}

// Opaque local identity, not a hardware address or a security token.
