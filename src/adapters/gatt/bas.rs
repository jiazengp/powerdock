//! Pure BAS 1.1 packet decoding and component aggregation.
use crate::battery::{Component, Kind, Level, Power, Reading, Value};

pub(super) fn level(data: &[u8]) -> Result<u8, String> {
    match data {
        [v] if *v <= 100 => Ok(*v),
        _ => Err("invalid_reply: BAS level".into()),
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Status {
    pub(super) identifier: Option<u16>,
    pub(super) level: Option<u8>,
    pub(super) power: Power,
    pub(super) external: Option<bool>,
    pub(super) present: bool,
    pub(super) ordinal: Option<Level>,
    pub(super) raw: u16,
}
pub(super) fn status(data: &[u8]) -> Result<Status, String> {
    if data.len() < 3 {
        return Err("invalid_reply: BLS header".into());
    }
    let flags = data[0];
    let expected = 3
        + usize::from(flags & 1 != 0) * 2
        + usize::from(flags & 2 != 0)
        + usize::from(flags & 4 != 0);
    if data.len() < expected || (flags & 0xf8 == 0 && data.len() != expected) {
        return Err("invalid_reply: BLS optional fields".into());
    }
    let state = u16::from_le_bytes([data[1], data[2]]);
    let present = state & 1 != 0;
    let mut offset = 3;
    let identifier = if flags & 1 != 0 {
        let n = u16::from_le_bytes([data[offset], data[offset + 1]]);
        offset += 2;
        Some(n)
    } else {
        None
    };
    let level = if flags & 2 != 0 {
        Some(level(&data[offset..offset + 1])?)
    } else {
        None
    };
    let power = if !present {
        Power::Unknown
    } else {
        match (state >> 5) & 3 {
            1 => Power::Charging,
            2 => Power::Battery,
            3 => Power::Inactive,
            _ => Power::Unknown,
        }
    };
    let wired = (state >> 1) & 3;
    let wireless = (state >> 3) & 3;
    let external = if wired == 1 || wireless == 1 {
        Some(true)
    } else if wired == 0 && wireless == 0 {
        Some(false)
    } else {
        None
    };
    Ok(Status {
        identifier,
        level,
        power,
        external,
        present,
        ordinal: match (state >> 7) & 3 {
            2 => Some(Level::Low),
            3 => Some(Level::Critical),
            _ => None,
        },
        raw: state,
    })
}
pub(super) fn cpf(data: &[u8]) -> Result<u16, String> {
    if data.len() != 7
        || data[0] != 4
        || data[1] != 0
        || data[2..4] != 0x27adu16.to_le_bytes()
        || data[4] != 1
    {
        return Err("invalid_reply: BAS presentation format".into());
    }
    Ok(u16::from_le_bytes([data[5], data[6]]))
}
pub(super) fn component_value(
    level: Option<u8>,
    status: Option<Status>,
    description: Option<u16>,
) -> Result<(Value, Power, Option<bool>, Option<u16>), String> {
    if let Some(s) = status {
        if let Some(id) = s.identifier
            && id != description.unwrap_or(0x0106)
        {
            return Err("invalid_reply: BAS Identifier/CPF conflict".into());
        }
        return Ok((
            if !s.present {
                Value::NoBattery
            } else {
                // Characteristics notify independently. The self-contained BLS
                // packet keeps its power/level together; a stale 2A19 is not a
                // conflicting atomic snapshot and must not tear down subscriptions.
                s.level.or(level).map_or_else(
                    || {
                        s.ordinal
                            .map_or(Value::Unknown, |level| Value::Ordinal { level })
                    },
                    Value::reported,
                )
            },
            s.power,
            s.external,
            s.identifier.or(description),
        ));
    }
    Ok((
        level.map_or(Value::Unknown, Value::reported),
        Power::Unknown,
        None,
        description,
    ))
}
pub(super) fn aggregate(components: &[Component], kind: Kind) -> Result<Reading, String> {
    if components.is_empty() || components.len() > 4 {
        return Err("invalid_reply: BAS component count".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    if components.iter().any(|c| !ids.insert(&c.id)) {
        return Err("invalid_reply: ambiguous BAS component identity".into());
    }
    let value = components
        .iter()
        .filter_map(|c| c.value.percent())
        .min()
        .map_or_else(
            || {
                if components.iter().all(|c| c.value == Value::NoBattery) {
                    Value::NoBattery
                } else {
                    components
                        .iter()
                        .filter_map(|c| {
                            if let Value::Ordinal { level } = c.value {
                                Some(level)
                            } else {
                                None
                            }
                        })
                        .min_by_key(|l| *l as u8)
                        .map_or(Value::Unknown, |level| Value::Ordinal { level })
                }
            },
            Value::reported,
        );
    let power = if components.len() == 1 {
        components[0].power
    } else {
        Power::Unknown
    };
    Ok(Reading::new(value, kind, power))
}
