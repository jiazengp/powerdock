//! Provider acquisition state owned by a single worker thread.
use super::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};

struct Apartment(bool);
impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

#[cfg(test)]
type Script = Box<dyn FnMut(&mut Sample, &AtomicU64, u64) -> Result<battery::Reading, String>>;

// Field order releases device sessions/subscriptions before the COM apartment.
pub(super) struct Reader {
    pub(super) link: system::Link,
    hidpp_identity: Option<Evidence>,
    hidpp_session: Option<hidpp::Session>,
    session: Option<mchose::Session>,
    #[cfg(test)]
    script: Option<Script>,
    apartment: Apartment,
}
impl Reader {
    pub(super) fn new() -> Self {
        let apartment = Apartment(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok());
        Self {
            session: None,
            hidpp_session: None,
            hidpp_identity: None,
            link: system::Link::default(),
            #[cfg(test)]
            script: None,
            apartment,
        }
    }
    #[cfg(test)]
    pub(super) fn scripted(
        script: impl FnMut(&mut Sample, &AtomicU64, u64) -> Result<battery::Reading, String> + 'static,
    ) -> Self {
        Self {
            session: None,
            hidpp_session: None,
            hidpp_identity: None,
            link: system::Link::default(),
            script: Some(Box::new(script)),
            apartment: Apartment(false),
        }
    }
    pub(super) fn reset(&mut self) {
        self.clear_sessions();
        self.hidpp_identity = None;
        self.link = system::Link::default();
    }
    pub(super) fn clear_sessions(&mut self) {
        self.session = None;
        self.hidpp_session = None;
    }
    pub(super) fn read(
        &mut self,
        target: &Target,
        sample: &mut Sample,
        epoch: &AtomicU64,
        expected: u64,
        policy: &settings::ProviderPreferences,
    ) -> Result<battery::Reading, String> {
        if !target.direct_reader_enabled(policy) {
            return Err("电量读取器已关闭".into());
        }
        #[cfg(test)]
        if let Some(script) = &mut self.script {
            return script(sample, epoch, expected);
        }
        match target {
            Target::SteelSeries(t) => {
                let result = if policy.source(Source::SteelSeries) {
                    steelseries::query(
                        &t.device,
                        epoch,
                        expected,
                        std::time::Instant::now() + Duration::from_secs(2),
                    )
                } else {
                    Err("unsupported: SteelSeries reader disabled".into())
                };
                if result.is_ok() {
                    sample.source = Source::SteelSeries;
                    self.link = system::Link::default();
                }
                self.fallback(result, t, sample, epoch, expected, policy)
            }
            Target::Hidpp(t) => {
                if let Some(identity) = &self.hidpp_identity {
                    sample.identity = identity.clone();
                }
                let mut result = Err("unsupported: HID++ reader disabled".into());
                if policy.source(Source::LogitechHidpp) {
                    result = if self.hidpp_session.is_none() {
                        match hidpp::Session::open(&t.device) {
                            Ok(session) => {
                                self.hidpp_session = Some(session);
                                Ok(())
                            }
                            Err(error) => Err(error),
                        }
                    } else {
                        Ok(())
                    }
                    .and_then(|()| {
                        self.hidpp_session
                            .as_ref()
                            .ok_or_else(|| String::from("HID++ session unavailable"))?
                            .query(epoch, expected)
                    })
                    .map(|observation| {
                        sample.source = Source::LogitechHidpp;
                        // A successful dedicated reader no longer needs the
                        // fallback's GATT subscriptions or health interval.
                        self.link = system::Link::default();
                        if matches!(
                            observation.identity,
                            Evidence::Unit(_) | Evidence::Strong(_)
                        ) {
                            self.hidpp_identity = Some(observation.identity.clone());
                            sample.identity = observation.identity;
                        }
                        observation.reading
                    });
                }
                self.fallback(result, t, sample, epoch, expected, policy)
            }
            Target::Mchose(d) => (|| {
                if self.session.is_none() {
                    self.session = Some(mchose::Session::open(d)?);
                }
                self.session
                    .as_ref()
                    .ok_or("设备会话不可用")?
                    .query(epoch, expected)
            })(),
            Target::Keyboard(d) => mchose::query_keyboard(d, epoch, expected),
            Target::Hid(d) => d.query(epoch, expected),
            Target::Windows(d) => {
                if self.apartment.0 {
                    d.query(
                        &mut self.link,
                        &mut sample.source,
                        &mut sample.connection,
                        epoch,
                        expected,
                        policy,
                    )
                } else {
                    Err("无法初始化系统设备访问".into())
                }
            }
        }
    }
    fn fallback<D>(
        &mut self,
        mut result: Result<battery::Reading, String>,
        target: &super::discovery::CompositeTarget<D>,
        sample: &mut Sample,
        epoch: &AtomicU64,
        expected: u64,
        policy: &settings::ProviderPreferences,
    ) -> Result<battery::Reading, String> {
        if result.is_err()
            && let Some(p) = &target.windows
            && self.apartment.0
        {
            result = p.query(
                &mut self.link,
                &mut sample.source,
                &mut sample.connection,
                epoch,
                expected,
                policy,
            );
        }
        if result.is_err()
            && policy.source(Source::HidStandard)
            && let Some(d) = &target.standard
        {
            sample.source = Source::HidStandard;
            result = d.query(epoch, expected);
        }
        result
    }
}
