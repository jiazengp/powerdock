mod tracking;
pub use tracking::Tracked;
use tracking::fresh;
mod selection;
pub use selection::{lowest, lowest_level, ordered};
#[cfg(test)]
mod observation_tests;
#[cfg(test)]
mod selection_tests;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub const FRESH_FOR: Duration = Duration::from_secs(300);

/// Protocol levels stay levels. Estimates never participate in percentage comparisons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "value_kind", rename_all = "snake_case")]
pub enum Value {
    ReportedPercent {
        percent: u8,
        resolution_percent: Option<u8>,
    },
    Ordinal {
        level: Level,
    },
    EstimatedPercent {
        percent: u8,
        model_version: u32,
    },
    Unknown,
    NoBattery,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Critical,
    Low,
    Medium,
    High,
    Full,
}
impl Value {
    fn normalized(self) -> Self {
        match self {
            Self::ReportedPercent { percent, .. } | Self::EstimatedPercent { percent, .. }
                if percent > 100 =>
            {
                Self::Unknown
            }
            Self::EstimatedPercent {
                model_version: 0, ..
            } => Self::Unknown,
            _ => self,
        }
    }
    pub fn reported(percent: u8) -> Self {
        if percent <= 100 {
            Self::ReportedPercent {
                percent,
                resolution_percent: None,
            }
        } else {
            Self::Unknown
        }
    }
    pub fn percent(self) -> Option<u8> {
        match self {
            Self::ReportedPercent { percent, .. } if percent <= 100 => Some(percent),
            _ => None,
        }
    }
    pub fn display(self) -> String {
        use crate::i18n::text;
        match self {
            Self::ReportedPercent { percent, .. } if percent <= 100 => format!("{percent}%"),
            Self::EstimatedPercent { percent, .. } if percent <= 100 => format!("≈{percent}%"),
            Self::Ordinal { level } => text(match level {
                Level::Critical => "BatteryCritical",
                Level::Low => "BatteryLow",
                Level::Medium => "BatteryMedium",
                Level::High => "BatteryHigh",
                Level::Full => "BatteryFull",
            }),
            Self::NoBattery => text("BatteryAbsent"),
            _ => "--".into(),
        }
    }
    fn sort_key(self) -> (u8, u8) {
        match self.normalized() {
            Self::ReportedPercent { percent, .. } => (0, percent),
            Self::Ordinal { level } => (1, level as u8),
            Self::EstimatedPercent { percent, .. } => (2, percent),
            _ => (3, 0),
        }
    }
    pub fn icon_fill(self) -> Option<u8> {
        match self.normalized() {
            Self::ReportedPercent { percent, .. } => Some(percent),
            Self::Ordinal { level } => Some((level as u8) * 25),
            _ => None,
        }
    }
}

pub const MAX_DEVICES: usize = 16;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Mouse,
    Keyboard,
    Headset,
    Gamepad,
    Phone,
    Other,
}
impl Kind {
    pub fn glyph(self) -> char {
        match self {
            Self::Mouse => '\u{E962}',
            Self::Keyboard => '\u{E765}',
            Self::Headset => '\u{E7F6}',
            Self::Gamepad => '\u{E7FC}',
            Self::Phone => '\u{E8EA}',
            Self::Other => '\u{E975}',
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Source {
    LogitechHidpp,
    HidStandard,
    Wgi,
    XInput,
    Mchose,
    MchoseUsb,
    MchoseKeyboard,
    Sennheiser,
    BluetoothGatt,
    Windows,
    Preview,
    Detected,
    Unknown,
    SteelSeries,
}
impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Self::SteelSeries => "SteelSeries 电量 · 实验",
            Self::LogitechHidpp => "Logitech HID++ 电量",
            Self::HidStandard => "标准 HID 电量",
            Self::Wgi => "Windows 控制器电量",
            Self::XInput => "XInput 控制器电量",
            Self::Mchose => "MCHOSE · 2.4G",
            Self::MchoseUsb => "MCHOSE · USB",
            Self::MchoseKeyboard => "MCHOSE 键盘 · 2.4G",
            Self::Sennheiser => "Sennheiser · 蓝牙直读",
            Self::BluetoothGatt => "蓝牙标准电量 · 直读",
            Self::Windows => "Windows 系统电量",
            Self::Preview => "模拟数据",
            Self::Detected => "USB 接收器 · 尚未适配",
            Self::Unknown => "等待有效读数",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Power {
    Battery,
    Charging,
    Full,
    Unknown,
    Inactive,
}

/// Visual state is independent of notification thresholds. Cached values are neutral.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BatteryTone {
    Normal,
    Low,
    Charging,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Reading {
    #[serde(flatten)]
    pub value: Value,
    pub kind: Kind,
    pub power: Power,
    pub connected: bool,
}
impl Reading {
    pub fn new(value: Value, kind: Kind, power: Power) -> Self {
        Self {
            value,
            kind,
            power,
            connected: true,
        }
    }
    pub fn percent(&self) -> Option<u8> {
        self.value.percent()
    }
    pub fn linked(&self) -> bool {
        self.connected
    }
    pub fn discharging(&self) -> bool {
        self.connected && self.power == Power::Battery
    }
    pub fn state(&self) -> &'static str {
        match self.power {
            Power::Battery => "电池供电",
            Power::Charging => "正在充电",
            Power::Full => "充电完成",
            Power::Unknown => "充电状态未知",
            Power::Inactive => "未活跃放电",
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Sample {
    pub id: String,
    pub name: String,
    pub source: Source,
    pub kind: Kind,
    pub connection: Option<bool>,
    pub at: u64,
    #[serde(skip)]
    pub clock: Instant,
    #[serde(skip)]
    pub identity: crate::identity::Evidence,
    #[serde(skip)]
    pub session: u64,
    #[serde(skip)]
    pub reader_epoch: u64,
    /// Battery receipt can precede the latest live connection/attempt check.
    #[serde(skip)]
    pub value_observation: Option<(u64, Instant)>,
    pub result: Result<Reading, String>,
    pub components: Vec<Component>,
}

/// Input IDs have already been canonicalized by the identity directory. Select
/// sources by battery receipt age, not by the latest connection check.
pub fn retain_preferred_samples(samples: &mut Vec<Sample>, now: u64) {
    samples.sort_by_key(|sample| {
        let (at, clock) = sample
            .value_observation
            .unwrap_or((sample.at, sample.clock));
        (
            sample.id.clone(),
            !sample
                .result
                .as_ref()
                .is_ok_and(|r| r.linked() && fresh(at, clock, now)),
            sample.source == Source::Windows,
            std::cmp::Reverse(clock),
        )
    });
    samples.dedup_by(|a, b| a.id == b.id);
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Component {
    pub id: String,
    pub role: &'static str,
    pub value: Value,
    pub power: Power,
    pub external_power: Option<bool>,
    pub identifier: Option<u16>,
    pub raw_power_state: Option<u16>,
}

pub fn key(namespace: &str, value: &str) -> String {
    let hash = value
        .to_lowercase()
        .bytes()
        .fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
        });
    format!("{namespace}-{hash:016x}")
}

pub fn preview_devices() -> Vec<Tracked> {
    [
        ("demo-mouse", "无线鼠标", 43, Kind::Mouse),
        ("demo-keyboard", "蓝牙键盘", 68, Kind::Keyboard),
        ("demo-headset", "蓝牙耳机", 18, Kind::Headset),
        ("demo-gamepad", "蓝牙手柄", 55, Kind::Gamepad),
    ]
    .into_iter()
    .map(|(id, name, percent, kind)| {
        Tracked::update(
            Sample {
                connection: None,
                id: id.into(),
                name: name.into(),
                source: Source::Preview,
                kind,
                at: crate::settings::now(),
                clock: std::time::Instant::now(),
                identity: crate::identity::Evidence::Unverified,
                session: 0,
                reader_epoch: 0,
                value_observation: None,
                components: Vec::new(),
                result: Ok(Reading {
                    value: Value::reported(percent),
                    kind,
                    power: if id == "demo-keyboard" {
                        Power::Charging
                    } else {
                        Power::Battery
                    },
                    connected: true,
                }),
            },
            None,
        )
    })
    .collect()
}
