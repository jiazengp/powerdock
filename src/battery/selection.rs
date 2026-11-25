//! Presentation ordering and minimum selection keep distinct policy contracts.
use super::{Kind, Power, Reading, Tracked, Value};

/// List ordering never changes tray selection. Unknown values stay last in both directions.
pub fn ordered(
    devices: &[Tracked],
    by: crate::settings::SortBy,
    order: crate::settings::SortOrder,
    now: u64,
) -> Vec<&Tracked> {
    use crate::settings::{SortBy, SortOrder};
    use std::cmp::Ordering;
    let rank = |device: &Tracked| -> Option<u8> {
        match by {
            SortBy::Battery => device.reading(now).and_then(Reading::percent),
            SortBy::Type => match device.kind {
                Kind::Mouse => Some(0),
                Kind::Keyboard => Some(1),
                Kind::Headset => Some(2),
                Kind::Gamepad => Some(3),
                Kind::Phone => Some(4),
                Kind::Other => None,
            },
            SortBy::Charging => device.reading(now).and_then(|r| match r.power {
                Power::Charging => Some(0),
                Power::Battery => Some(1),
                Power::Full => Some(2),
                Power::Unknown => None,
                Power::Inactive => None,
            }),
        }
    };
    let mut result: Vec<_> = devices.iter().collect();
    result.sort_by(|a, b| {
        if by == SortBy::Battery {
            let (ag, ar) = a.reading(now).map_or((3, 0), |r| r.value.sort_key());
            let (bg, br) = b.reading(now).map_or((3, 0), |r| r.value.sort_key());
            return ag
                .cmp(&bg)
                .then_with(|| {
                    if order == SortOrder::Desc {
                        br.cmp(&ar)
                    } else {
                        ar.cmp(&br)
                    }
                })
                .then_with(|| a.id.cmp(&b.id));
        }
        match (rank(a), rank(b)) {
            (Some(left), Some(right)) => {
                if order == SortOrder::Desc {
                    right.cmp(&left)
                } else {
                    left.cmp(&right)
                }
            }
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
        .then_with(|| a.id.cmp(&b.id))
    });
    result
}

/// Compare only available observations. Equal readings retain the current icon.
pub fn lowest(devices: &[Tracked], current: Option<&str>, now: u64) -> Option<String> {
    let available: Vec<_> = devices
        .iter()
        .filter(|d| d.reading(now).is_some())
        .collect();
    let minimum = available
        .iter()
        .filter_map(|d| d.last.as_ref().and_then(|(r, _)| r.percent()))
        .min()?;
    if let Some(id) = current
        && available.iter().any(|d| {
            d.id == id
                && d.last
                    .as_ref()
                    .is_some_and(|(r, _)| r.percent() == Some(minimum))
        })
    {
        return Some(id.to_owned());
    }
    available
        .into_iter()
        .find(|d| {
            d.last
                .as_ref()
                .is_some_and(|(r, _)| r.percent() == Some(minimum))
        })
        .map(|d| d.id.clone())
}
pub fn lowest_level(devices: &[Tracked], current: Option<&str>, now: u64) -> Option<String> {
    let candidates: Vec<_> = devices
        .iter()
        .filter_map(|d| {
            if let Value::Ordinal { level } = d.reading(now)?.value {
                Some((d.id.clone(), level as u8))
            } else {
                None
            }
        })
        .collect();
    let min = candidates.iter().map(|(_, v)| *v).min()?;
    candidates
        .iter()
        .find(|(id, v)| *v == min && Some(id.as_str()) == current)
        .or_else(|| candidates.iter().find(|(_, v)| *v == min))
        .map(|(id, _)| id.clone())
}
