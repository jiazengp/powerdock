//! Descriptor qualification, null/signed ranges and Battery Strength precision.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReportType {
    Input,
    Feature,
}
impl ReportType {
    pub(super) fn native(self) -> HIDP_REPORT_TYPE {
        match self {
            Self::Input => HidP_Input,
            Self::Feature => HidP_Feature,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Field {
    pub(super) report_type: ReportType,
    pub(super) report_id: u8,
    pub(super) report_bytes: u16,
    pub(super) link: u16,
    pub(super) bits: u16,
    pub(super) min: i32,
    pub(super) max: i32,
    pub(super) has_null: bool,
}
impl Field {
    pub(super) fn from_cap(
        cap: &HIDP_VALUE_CAPS,
        report_type: ReportType,
        bytes: u16,
    ) -> Option<Self> {
        let contains_usage = unsafe {
            if cap.IsRange {
                let range = cap.Anonymous.Range;
                range.UsageMin <= USAGE && USAGE <= range.UsageMax
            } else {
                cap.Anonymous.NotRange.Usage == USAGE
            }
        };
        // Constant/relative data and repeated scalar usages are not battery state.
        // A usage range has one value per usage; a non-range ReportCount>1 is
        // an array with potentially several batteries, which this reader cannot
        // assign to physical components safely.
        if cap.UsagePage != PAGE
            || !contains_usage
            || cap.BitField & 3 != 2
            || !cap.IsAbsolute
            || cap.IsAlias
            || cap.ReportCount == 0
            || (!cap.IsRange && cap.ReportCount != 1)
            || !(1..=32).contains(&cap.BitSize)
            || !(2..=MAX_REPORT).contains(&bytes)
        {
            return None;
        }
        Some(Self {
            report_type,
            report_id: cap.ReportID,
            report_bytes: bytes,
            link: cap.LinkCollection,
            bits: cap.BitSize,
            min: cap.LogicalMin,
            max: cap.LogicalMax,
            has_null: cap.HasNull,
        })
    }

    pub(super) fn validate_report(&self, report: &[u8]) -> Result<(), String> {
        // Never pad short reports: padded zeroes can manufacture an empty battery.
        if report.len() != usize::from(self.report_bytes) {
            return Err("invalid_reply: HID 电量报告长度无效".into());
        }
        if report.first().copied() != Some(self.report_id) {
            return Err("invalid_reply: HID 电量报告 ID 不匹配".into());
        }
        Ok(())
    }

    pub(super) fn value(&self, raw: u32, status: NTSTATUS) -> Result<Value, String> {
        if status == HIDP_STATUS_NULL && self.has_null {
            return Ok(Value::Unknown);
        }
        if status != HIDP_STATUS_SUCCESS {
            return Err("invalid_reply: HID 电量字段无法解析".into());
        }
        if self.min >= self.max || !(1..=32).contains(&self.bits) {
            return Err("unsupported: HID 电量逻辑范围无效".into());
        }
        let mask = (1u64 << self.bits) - 1;
        let raw = u64::from(raw);
        if raw & !mask != 0 {
            return Err("invalid_reply: HID 电量超过声明位宽".into());
        }
        let signed = self.min < 0;
        let (representable_min, representable_max) = if signed {
            (-(1i64 << (self.bits - 1)), (1i64 << (self.bits - 1)) - 1)
        } else {
            (0, mask as i64)
        };
        if i64::from(self.min) < representable_min || i64::from(self.max) > representable_max {
            return Err("unsupported: HID 电量范围超过声明位宽".into());
        }
        // HidP_GetUsageValue returns unscaled bits without sign extension.
        let value = if signed && raw & (1u64 << (self.bits - 1)) != 0 {
            raw as i64 - (1i64 << self.bits)
        } else {
            raw as i64
        };
        let (min, max) = (i64::from(self.min), i64::from(self.max));
        if value < min || value > max {
            return if self.has_null {
                Ok(Value::Unknown)
            } else {
                Err("invalid_reply: HID 电量超出逻辑范围".into())
            };
        }
        if self.min == 0 && self.max == 3 {
            return Ok(Value::Ordinal {
                level: match value {
                    0 => Level::Critical,
                    1 => Level::Low,
                    2 => Level::Medium,
                    _ => Level::Full,
                },
            });
        }
        if self.min == 0 && self.max == 100 {
            return Ok(Value::reported(value as u8));
        }
        let span = max - min;
        let percent = ((value - min) * 100 + span / 2) / span;
        // The model displays integer percentages; retain coarser source steps.
        let resolution = ((100 + span - 1) / span).clamp(1, 100) as u8;
        Ok(Value::ReportedPercent {
            percent: percent as u8,
            resolution_percent: Some(resolution),
        })
    }

    pub(super) fn decode(
        &self,
        preparsed: PHIDP_PREPARSED_DATA,
        report: &[u8],
    ) -> Result<Value, String> {
        self.validate_report(report)?;
        let mut raw = 0;
        let status = unsafe {
            HidP_GetUsageValue(
                self.report_type.native(),
                PAGE,
                self.link,
                USAGE,
                &mut raw,
                preparsed,
                report.as_ptr(),
                report.len() as u32,
            )
        };
        self.value(raw, status)
    }
}
