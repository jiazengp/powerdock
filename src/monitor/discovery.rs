//! Qualification, transport association, stable ordering and budget-visible discovery.
use super::*;

/// Discovery metadata only: admission to the read budget is not proof of connection.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Candidate {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub sources: Vec<Source>,
    pub scheduling: &'static str,
}
pub(super) struct Discovery {
    pub(super) targets: Vec<Target>,
    pub(super) catalog: Vec<Candidate>,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct CompositeTarget<D> {
    pub(super) device: D,
    pub(super) windows: Option<system::Peripheral>,
    pub(super) standard: Option<hid_standard::Device>,
}
pub(super) type HidppTarget = CompositeTarget<hidpp::Device>;
pub(super) type SteelSeriesTarget = CompositeTarget<device::Device>;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum Target {
    SteelSeries(Box<SteelSeriesTarget>),
    Hidpp(Box<HidppTarget>),
    Hid(hid_standard::Device),
    Mchose(device::Device),
    Windows(system::Peripheral),
    Keyboard(device::Device),
}
impl Target {
    pub(super) fn policy_changed(
        &self,
        previous: &settings::ProviderPreferences,
        next: &settings::ProviderPreferences,
    ) -> bool {
        match self {
            Self::Mchose(d) => previous.mouse_device_enabled(d) != next.mouse_device_enabled(d),
            Self::Keyboard(d) => {
                previous.keyboard_device_enabled(d) != next.keyboard_device_enabled(d)
            }
            _ => previous.changed_for(next, &self.sources()),
        }
    }
    pub(super) fn direct_reader_enabled(&self, policy: &settings::ProviderPreferences) -> bool {
        match self {
            Self::Keyboard(d) => policy.keyboard_device_enabled(d),
            Self::Mchose(d) => policy.mouse_device_enabled(d),
            Self::Hid(_) => policy.source(Source::HidStandard),
            _ => true, // Composite/Windows targets apply per-source fallback policy below.
        }
    }

    pub(super) fn sources(&self) -> Vec<Source> {
        match self {
            Self::SteelSeries(t) => composite_sources(t, Source::SteelSeries),
            Self::Hidpp(t) => composite_sources(t, Source::LogitechHidpp),
            Self::Hid(_) => vec![Source::HidStandard],
            Self::Windows(d) => d.sources(),
            _ => vec![self.sample().source],
        }
    }
    pub(super) fn matches_interface(&self, interfaces: &[String]) -> bool {
        match self {
            Self::SteelSeries(t) => {
                steelseries::matches_interface(&t.device, interfaces)
                    || composite_hid_changed(t, interfaces)
            }
            Self::Hidpp(t) => {
                t.device.matches_interface(interfaces) || composite_hid_changed(t, interfaces)
            }
            Self::Hid(d) => d.matches_interface(interfaces),
            Self::Mchose(d) | Self::Keyboard(d) => {
                device::matches_interface_path(&d.path, interfaces)
            }
            Self::Windows(d) => d
                .standard_hid
                .as_ref()
                .is_some_and(|h| h.matches_interface(interfaces)),
        }
    }
    pub(super) fn id(&self) -> String {
        match self {
            Self::SteelSeries(t) => steelseries::id(&t.device),
            Self::Hidpp(t) => t
                .windows
                .as_ref()
                .map_or_else(|| t.device.id.clone(), |p| p.id.clone()),
            Self::Hid(d) => d.id.clone(),
            Self::Mchose(d) | Self::Keyboard(d) => mchose::id(d),
            Self::Windows(d) => d.id.clone(),
        }
    }
    pub(super) fn sample(&self) -> Sample {
        Sample {
            connection: if matches!(self, Self::Mchose(d) if mchose::profiles::mouse_wired(d)) {
                Some(true)
            } else {
                None
            },
            id: self.id(),
            name: match self {
                Self::SteelSeries(t) => steelseries::profile(&t.device)
                    .map_or_else(|| t.device.product.clone(), |p| p.name.into()),
                Self::Hidpp(t) => t
                    .windows
                    .as_ref()
                    .map_or_else(|| t.device.name.clone(), |p| p.name.clone()),
                Self::Hid(d) => d.name.clone(),
                Self::Mchose(d) | Self::Keyboard(d) => d.product.clone(),
                Self::Windows(d) => d.name.clone(),
            },
            source: match self {
                Self::SteelSeries(_) => Source::SteelSeries,
                Self::Hidpp(_) => Source::LogitechHidpp,
                Self::Hid(_) => Source::HidStandard,
                Self::Mchose(d) if mchose::profiles::mouse_wired(d) => Source::MchoseUsb,
                Self::Mchose(_) => Source::Mchose,
                Self::Windows(_) => Source::Windows,
                Self::Keyboard(_) => Source::MchoseKeyboard,
            },
            kind: match self {
                Self::SteelSeries(_) => Kind::Mouse,
                Self::Hidpp(t) => t.windows.as_ref().map_or(t.device.kind, |p| p.kind),
                Self::Hid(d) => d.kind,
                Self::Mchose(_) => Kind::Mouse,
                Self::Keyboard(_) => Kind::Keyboard,
                Self::Windows(d) => d.kind,
            },
            at: settings::now(),
            clock: std::time::Instant::now(),
            session: 0,
            reader_epoch: 0,
            value_observation: None,
            components: Vec::new(),
            identity: match self {
                Self::SteelSeries(t) => {
                    if steelseries::profile(&t.device).is_some_and(|p| p.wired) {
                        t.device
                            .container
                            .as_ref()
                            .map_or(Evidence::Route(self.id()), |c| {
                                Evidence::Strong(battery::key("container", c))
                            })
                    } else {
                        Evidence::Route(self.id())
                    }
                }
                Self::Hidpp(t) => t
                    .windows
                    .as_ref()
                    .and_then(|p| p.container.as_ref())
                    .map_or_else(
                        || Evidence::Route(self.id()),
                        |c| Evidence::Strong(battery::key("container", c)),
                    ),
                Self::Hid(d) => {
                    if d.associated {
                        d.container.as_ref().map_or(Evidence::Unverified, |c| {
                            Evidence::Strong(battery::key("container", c))
                        })
                    } else {
                        Evidence::Route(self.id())
                    }
                }
                Self::Mchose(d) if mchose::profiles::mouse_wired(d) => {
                    d.container.as_ref().map_or(Evidence::Unverified, |c| {
                        Evidence::Strong(battery::key("container", c))
                    })
                }
                Self::Mchose(_) | Self::Keyboard(_) => Evidence::Route(self.id()),
                Self::Windows(d) => d.container.as_ref().map_or(Evidence::Unverified, |c| {
                    Evidence::Strong(battery::key("container", c))
                }),
            },
            result: Err("正在查询电量…".into()),
        }
    }
}

fn composite_hid_changed<D>(target: &CompositeTarget<D>, interfaces: &[String]) -> bool {
    target
        .standard
        .iter()
        .chain(
            target
                .windows
                .iter()
                .filter_map(|p| p.standard_hid.as_ref()),
        )
        .any(|hid| hid.matches_interface(interfaces))
}
pub(super) fn discover_with(
    policy: &settings::ProviderPreferences,
    watcher: Option<&crate::discovery::Watchers>,
) -> Result<Discovery, String> {
    let native_result = device::enumerate();
    let system_result = system::enumerate_with(watcher).or_else(|error| {
        if watcher.is_some() {
            settings::log_error(&error);
            system::enumerate()
        } else {
            Err(error)
        }
    });
    if let (Err(a), Err(b)) = (&native_result, &system_result) {
        return Err(format!("{a}；{b}"));
    }
    let paired = system_result.as_ref().ok().cloned().unwrap_or_default();
    let hid = native_result.unwrap_or_default();
    let native: Vec<_> = hid
        .iter()
        .filter(|d| policy.mouse_device_enabled(d))
        .cloned()
        .collect();
    let containers: Vec<_> = native
        .iter()
        .filter_map(|d| d.container.as_deref())
        .collect();
    let mut targets: Vec<_> = native.iter().cloned().map(Target::Mchose).collect();
    targets.extend(
        system_result
            .unwrap_or_default()
            .into_iter()
            .filter(|d| {
                d.container
                    .as_deref()
                    .is_none_or(|c| !containers.contains(&c))
            })
            .map(Target::Windows),
    );
    if policy.source(Source::HidStandard) {
        let existing: Vec<_> = native.iter().filter_map(|d| d.container.as_ref()).collect();
        let standard = hid_standard::enumerate_from(&hid)?
            .into_iter()
            .filter(|d| d.feature_readable())
            .filter(|d| d.container.as_ref().is_none_or(|c| !existing.contains(&c)))
            .map(|mut d| {
                d.associated = d
                    .container
                    .as_ref()
                    .is_some_and(|c| paired.iter().any(|p| p.container.as_ref() == Some(c)));
                if !d.associated {
                    d.id = battery::key("hid-route", &d.id);
                }
                d
            })
            .collect();
        attach_standard_hid(&mut targets, standard);
    }
    targets.extend(
        hid.iter()
            .filter(|d| mchose::profiles::mchose_keyboard(d))
            .filter(|d| policy.keyboard_device_enabled(d))
            .cloned()
            .map(Target::Keyboard),
    );
    if policy.source(Source::LogitechHidpp) {
        attach_hidpp(&mut targets, hidpp::enumerate_from(&hid));
    }
    if policy.source(Source::SteelSeries) {
        attach_steelseries(&mut targets, hid.clone());
    }
    let catalog = prioritize_targets(&mut targets, policy);
    Ok(Discovery { targets, catalog })
}

// An OS-associated Feature reader is another source of the same actor, not
// another candidate. Keep the Windows fallback and the active-device budget.
pub(super) fn attach_standard_hid(targets: &mut Vec<Target>, devices: Vec<hid_standard::Device>) {
    for hid in &devices {
        if hid.associated {
            let same = devices
                .iter()
                .filter(|d| d.associated && d.container == hid.container)
                .count();
            // Multiple collections can describe distinct batteries. Do not pick
            // one by path order or publish duplicate physical devices.
            if same != 1 {
                continue;
            }
            if let Some(Target::Windows(peripheral)) = targets.iter_mut().find(|t|
                matches!(t, Target::Windows(p) if p.container.is_some() && p.container == hid.container)) {
                peripheral.standard_hid = Some(hid.clone());
                continue;
            }
        }
        targets.push(Target::Hid(hid.clone()));
    }
}
fn composite_sources<D>(target: &CompositeTarget<D>, primary: Source) -> Vec<Source> {
    let mut sources = vec![primary];
    if let Some(p) = &target.windows {
        sources.extend(p.sources());
    }
    if target.standard.is_some() {
        sources.push(Source::HidStandard);
    }
    sources
}
fn attach_composite<D>(
    targets: &mut Vec<Target>,
    device: D,
    container_id: Option<String>,
) -> CompositeTarget<D> {
    let same_container =
        |container: &Option<String>| container_id.is_some() && *container == container_id;
    let windows = targets
        .iter()
        .position(|t| matches!(t,Target::Windows(p) if same_container(&p.container)))
        .map(|i| match targets.remove(i) {
            Target::Windows(p) => p,
            _ => unreachable!(),
        });
    let mut standards = Vec::new();
    targets.retain(|t| {
        if let Target::Hid(d) = t
            && same_container(&d.container)
        {
            standards.push(d.clone());
            return false;
        }
        true
    });
    let standard = if standards.len() == 1 {
        standards.pop()
    } else {
        None
    };
    CompositeTarget {
        device,
        windows,
        standard,
    }
}
pub(super) fn attach_hidpp(targets: &mut Vec<Target>, devices: Vec<hidpp::Device>) {
    for device in devices.into_iter().filter(|d| d.monitor_eligible()) {
        let container = device.container.clone();
        let target = attach_composite(targets, device, container);
        targets.push(Target::Hidpp(Box::new(target)));
    }
}
pub(super) fn attach_steelseries(targets: &mut Vec<Target>, devices: Vec<device::Device>) {
    for device in steelseries::enumerate_from(&devices) {
        let container = device.container.clone();
        let target = attach_composite(targets, device, container);
        targets.push(Target::SteelSeries(Box::new(target)));
    }
}
pub(super) fn prioritize_targets(
    targets: &mut Vec<Target>,
    policy: &settings::ProviderPreferences,
) -> Vec<Candidate> {
    targets.retain(|target| target.sources().iter().any(|source| policy.source(*source)));
    targets.sort_by_key(|t| {
        (
            match t {
                Target::Mchose(d) if mchose::profiles::mouse_wired(d) => 0,
                Target::Mchose(_) => 1,
                Target::Hidpp(_) | Target::Hid(_) | Target::SteelSeries(_) => 3,
                Target::Keyboard(_) => 2,
                Target::Windows(d) if d.connected_hint => 3,
                Target::Windows(d)
                    if policy.source(Source::Sennheiser)
                        && d.sources().contains(&Source::Sennheiser) =>
                {
                    4
                }
                Target::Windows(d) if d.kind != Kind::Other => 5,
                Target::Windows(_) => 6,
            },
            t.id(),
        )
    });
    let mut seen = std::collections::HashSet::new();
    targets.retain(|t| seen.insert(t.id()));
    let catalog = targets
        .iter()
        .enumerate()
        .map(|(index, t)| {
            let sample = t.sample();
            Candidate {
                id: sample.id,
                name: sample.name.chars().take(48).collect(),
                kind: sample.kind,
                sources: t.sources(),
                scheduling: if index < battery::MAX_DEVICES {
                    "active"
                } else {
                    "budget_deferred"
                },
            }
        })
        .collect();
    if targets.len() > battery::MAX_DEVICES {
        crate::diagnostics::record(
            "budget_deferred",
            serde_json::json!({"discovered_candidates":targets.len(),"limit":battery::MAX_DEVICES}),
        );
    }
    targets.truncate(battery::MAX_DEVICES);
    catalog
}
