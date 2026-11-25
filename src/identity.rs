//! Local evidence directory. Names and VID/PID are never identity evidence.
use crate::{battery, settings};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const LIMIT: usize = 256 * 1024;
#[derive(Clone, Debug, Default)]
pub enum Evidence {
    #[default]
    Unverified,
    Strong(String),
    /// A protocol unit identifier is exclusive: a reused route must not join two units.
    Unit(String),
    Route(String),
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Record {
    unit: Option<String>,
    aliases: BTreeSet<String>,
    evidence: BTreeSet<String>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Route {
    incarnation: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Directory {
    version: u8,
    records: BTreeMap<String, Record>,
    routes: BTreeMap<String, Route>,
    ambiguous_units: BTreeSet<String>,
    ambiguous_proofs: BTreeSet<String>,
    #[serde(skip)]
    live_routes: BTreeMap<String, (u64, String)>,
}
impl Default for Directory {
    fn default() -> Self {
        Self {
            version: 1,
            records: BTreeMap::new(),
            routes: BTreeMap::new(),
            ambiguous_units: BTreeSet::new(),
            ambiguous_proofs: BTreeSet::new(),
            live_routes: BTreeMap::new(),
        }
    }
}
pub struct Resolution {
    pub id: String,
    pub changed: bool,
}
#[derive(Default)]
pub struct Reconciled {
    pub endpoints: BTreeMap<String, Vec<String>>,
    pub blocked: BTreeSet<String>,
    pub error: Option<String>,
}
fn temporary_id(endpoint: &str, session: u64) -> String {
    static PROCESS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let process = PROCESS.get_or_init(|| {
        unsafe { windows::Win32::System::Com::CoCreateGuid() }
            .map(|id| format!("{id:?}"))
            .unwrap_or_else(|_| {
                format!("{}:{:?}", std::process::id(), std::time::SystemTime::now())
            })
    });
    battery::key("session", &format!("{process}:{endpoint}:{session}"))
}
/// Commit associations for the whole batch before migrating settings and selecting
/// observations. Failed migration must qualify the physical id, not one endpoint.
pub fn reconcile(
    directory: &mut Option<Directory>,
    config: &mut settings::Settings,
    samples: &mut [battery::Sample],
    mut save_directory: impl FnMut(&Directory) -> Result<(), String>,
    mut save_settings: impl FnMut(&settings::Settings) -> Result<(), String>,
) -> Reconciled {
    let mut result = Reconciled::default();
    let endpoints: Vec<_> = samples.iter().map(|s| s.id.clone()).collect();
    let mut next = directory.clone();
    let resolution = (|| {
        let next = next.as_mut().ok_or("设备目录不可用，提醒已暂停")?;
        let mut changed = false;
        // Unit evidence can teach a conflict during this very batch. Resolve it
        // before any lower-quality observation can select historical settings.
        for unit_phase in [true, false] {
            for sample in samples
                .iter_mut()
                .filter(|s| s.session != 0 && matches!(s.identity, Evidence::Unit(_)) == unit_phase)
            {
                let resolved = next.resolve(&sample.id, &sample.identity, sample.session)?;
                sample.id = resolved.id;
                changed |= resolved.changed;
            }
        }
        // A later endpoint may have joined two earlier records. Normalize again
        // against the final directory so no obsolete canonical survives this batch.
        for sample in samples.iter_mut() {
            sample.id = next.canonical(&sample.id);
        }
        if changed {
            save_directory(next)?;
        }
        Ok::<_, String>(())
    })();
    if let Err(error) = resolution {
        result.error = Some(error);
        for (sample, endpoint) in samples.iter_mut().zip(&endpoints) {
            sample.id = temporary_id(endpoint, sample.session);
            result.blocked.insert(sample.id.clone());
        }
    } else if let Some(next) = next {
        let mut updated = config.clone();
        for id in samples.iter().map(|s| &s.id).collect::<BTreeSet<_>>() {
            {
                let aliases = next.aliases(id);
                if aliases.iter().any(|a| {
                    config.device_thresholds.contains_key(a) || config.device_alerts.contains_key(a)
                }) {
                    result.blocked.insert(id.clone());
                }
                updated.merge_device_aliases(id, &aliases);
            }
        }
        *directory = Some(next);
        if !result.blocked.is_empty() {
            match save_settings(&updated) {
                Ok(()) => {
                    *config = updated;
                    result.blocked.clear();
                }
                Err(error) => result.error = Some(format!("设备提醒设置迁移未保存：{error}")),
            }
        }
    }
    for (sample, endpoint) in samples.iter().zip(endpoints) {
        result
            .endpoints
            .entry(sample.id.clone())
            .or_default()
            .push(endpoint);
    }
    result
}
impl Directory {
    pub fn load() -> Result<Self, String> {
        Self::load_from(&settings::root())
    }
    fn load_from(root: &Path) -> Result<Self, String> {
        let path = root.join("devices.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .map_err(|e| e.to_string())?
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > LIMIT {
            return Err("设备目录超过大小限制".into());
        }
        let directory: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        directory.validate()?;
        Ok(directory)
    }
    fn validate(&self) -> Result<(), String> {
        let valid = settings::device_id_valid;
        if self.version != 1
            || self.records.len() > 256
            || self.routes.len() > 256
            || self.ambiguous_units.len() > 256
            || self.ambiguous_proofs.len() > 256
            || self.ambiguous_proofs.iter().any(|proof| !valid(proof))
            || self.ambiguous_units.iter().any(|id| !valid(id))
            || self.records.iter().any(|(id, r)| {
                !valid(id)
                    || r.unit
                        .as_ref()
                        .is_some_and(|unit| !valid(unit) || !r.evidence.contains(unit))
                    || r.aliases.len() > 32
                    || r.evidence.len() > 32
                    || r.aliases.iter().chain(&r.evidence).any(|v| !valid(v))
            })
            || self.routes.keys().any(|v| !valid(v))
        {
            return Err("设备目录无效".into());
        }
        let mut seen = BTreeSet::new();
        for (id, r) in &self.records {
            for alias in std::iter::once(id).chain(&r.aliases) {
                if !seen.insert(alias) {
                    return Err("设备目录别名冲突".into());
                }
            }
        }
        Ok(())
    }
    pub fn save(&self) -> Result<(), String> {
        self.save_to(&settings::root())
    }
    fn save_to(&self, root: &Path) -> Result<(), String> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        settings::atomic_write(root, "devices", &bytes, LIMIT)
    }
    fn aliases(&self, id: &str) -> Vec<String> {
        self.records
            .get(id)
            .map_or_else(Vec::new, |r| r.aliases.iter().cloned().collect())
    }
    pub fn canonical(&self, id: &str) -> String {
        self.records
            .iter()
            .find(|(canonical, r)| canonical.as_str() == id || r.aliases.contains(id))
            .map_or_else(|| id.to_owned(), |(canonical, _)| canonical.clone())
    }
    fn alias_is_ambiguous(&self, alias: &str) -> bool {
        self.ambiguous_units.contains(alias)
            || self.ambiguous_proofs.contains(alias)
            || self.records.iter().any(|(id, record)| {
                (id == alias || record.aliases.contains(alias))
                    && (record
                        .evidence
                        .iter()
                        .any(|p| self.ambiguous_proofs.contains(p))
                        || std::iter::once(id)
                            .chain(&record.aliases)
                            .any(|a| self.ambiguous_units.contains(a)))
            })
    }
    fn resolve_route(
        &mut self,
        observed: &str,
        route: &str,
        session: u64,
    ) -> Result<Resolution, String> {
        if let Some((active, id)) = self.live_routes.get(route)
            && *active == session
        {
            return Ok(Resolution {
                id: id.clone(),
                changed: false,
            });
        }
        if self.routes.len() >= 256 && !self.routes.contains_key(route) {
            return Err("设备路由目录已满".into());
        }
        let entry = self.routes.entry(route.to_owned()).or_default();
        let id = if entry.incarnation == 0 {
            observed.into()
        } else {
            battery::key("route", &format!("{route}:{}", entry.incarnation))
        };
        entry.incarnation = entry
            .incarnation
            .checked_add(1)
            .ok_or("设备路由世代已耗尽")?;
        self.live_routes
            .insert(route.to_owned(), (session, id.clone()));
        Ok(Resolution { id, changed: true })
    }
    /// A new worker means continuity is unproven for route-only devices. Busy and
    /// sleeping responses keep the same worker/session and never create an incarnation.
    pub fn resolve(
        &mut self,
        observed: &str,
        evidence: &Evidence,
        session: u64,
    ) -> Result<Resolution, String> {
        if !settings::device_id_valid(observed) {
            return Err("设备 ID 无效".into());
        }
        // Validate before applying the shared fence: lower-quality evidence must
        // not skip validation just because its endpoint is already ambiguous.
        match evidence {
            Evidence::Route(route) if !settings::device_id_valid(route) => {
                return Err("路由 ID 无效".into());
            }
            Evidence::Strong(proof) | Evidence::Unit(proof)
                if !settings::device_id_valid(proof) =>
            {
                return Err("身份证据无效".into());
            }
            _ => {}
        }
        let ambiguous_context = match evidence {
            Evidence::Strong(proof) => self.alias_is_ambiguous(proof),
            Evidence::Route(route) => {
                self.alias_is_ambiguous(route)
                    || self
                        .live_routes
                        .get(route)
                        .is_some_and(|(_, id)| self.alias_is_ambiguous(id))
            }
            _ => false,
        };
        if !matches!(evidence, Evidence::Unit(_))
            && (self.alias_is_ambiguous(observed) || ambiguous_context)
        {
            // Fence every non-unit source before canonical lookup or route-cache
            // reuse. A derived endpoint also avoids the first-incarnation alias trap.
            let learned = self.ambiguous_units.insert(observed.into());
            let route = battery::key("unproven-unit-route", observed);
            let mut resolution = self.resolve_route(&route, &route, session)?;
            self.validate()?;
            resolution.changed |= learned;
            return Ok(resolution);
        }
        match evidence {
            Evidence::Unverified => Ok(Resolution {
                id: self.canonical(observed),
                changed: false,
            }),
            Evidence::Route(route) => self.resolve_route(observed, route, session),
            Evidence::Strong(proof) | Evidence::Unit(proof) => {
                let exclusive = matches!(evidence, Evidence::Unit(_));
                let mut changed_ambiguity = false;
                if exclusive {
                    for (id, record) in &self.records {
                        if (id == observed || record.aliases.contains(observed))
                            && record.unit.as_ref().is_some_and(|unit| unit != proof)
                        {
                            for alias in std::iter::once(id).chain(&record.aliases) {
                                changed_ambiguity |= self.ambiguous_units.insert(alias.clone());
                            }
                            changed_ambiguity |= self.ambiguous_units.insert(observed.into());
                            // The conflicting route was linked through these non-unit
                            // proofs. Fence their other aliases too, not only this ID.
                            for prior in record
                                .evidence
                                .iter()
                                .filter(|p| record.unit.as_ref() != Some(*p))
                            {
                                changed_ambiguity |= self.ambiguous_proofs.insert(prior.clone());
                            }
                        }
                    }
                }
                let owners: Vec<_> = self
                    .records
                    .iter()
                    .filter(|(id, r)| {
                        (!exclusive || r.unit.as_ref().is_none_or(|unit| unit == proof))
                            && (id.as_str() == observed
                                || r.aliases.contains(observed)
                                || r.evidence.contains(proof))
                    })
                    .map(|(id, _)| id.clone())
                    .collect();
                // Prefer an already published id. If two published records gain a strong
                // association, retain the lexical first id to make discovery order irrelevant.
                let units: BTreeSet<_> = owners
                    .iter()
                    .filter_map(|id| self.records[id].unit.as_ref())
                    .collect();
                if units.len() > 1 {
                    return Err("设备身份证据指向不同 unit，拒绝合并".into());
                }
                let id = owners.first().cloned().unwrap_or_else(|| {
                    if exclusive {
                        proof.clone()
                    } else {
                        observed.into()
                    }
                });
                let before = serde_json::to_vec(&self.records).map_err(|e| e.to_string())?;
                let mut record = self.records.remove(&id).unwrap_or_default();
                for old in owners.into_iter().filter(|v| v != &id) {
                    if let Some(r) = self.records.remove(&old) {
                        record.aliases.insert(old);
                        record.aliases.extend(r.aliases);
                        record.evidence.extend(r.evidence);
                        record.unit = record.unit.or(r.unit);
                    }
                }
                // A new unit can arrive on a route still owned by an older unit.
                // Preserve that old device and its overrides; do not steal its alias.
                let occupied = self
                    .records
                    .iter()
                    .any(|(id, r)| id == observed || r.aliases.contains(observed));
                if observed != id && !occupied {
                    record.aliases.insert(observed.into());
                }
                record.evidence.insert(proof.clone());
                if exclusive {
                    record.unit = Some(proof.clone());
                }
                self.records.insert(id.clone(), record);
                self.validate()?;
                let changed = changed_ambiguity
                    || before != serde_json::to_vec(&self.records).map_err(|e| e.to_string())?;
                Ok(Resolution { id, changed })
            }
        }
    }
}

#[cfg(test)]
mod tests;
