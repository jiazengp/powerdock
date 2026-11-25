use serde::{Deserialize, Serialize};

pub const BUSY: &str = "鼠标暂未响应，移动鼠标后会自动刷新";
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Busy,
    Invalid(&'static str),
}
impl From<&'static str> for ParseError {
    fn from(value: &'static str) -> Self {
        Self::Invalid(value)
    }
}
impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Busy => BUSY,
            Self::Invalid(message) => message,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Reading {
    pub percent: u8,
    pub charge_code: u8,
    pub connection_code: u8,
    pub product_field: u16,
}

impl Reading {
    pub fn linked(&self) -> bool {
        matches!(self.connection_code, 1 | 2)
    }
}
impl From<Reading> for crate::battery::Reading {
    fn from(value: Reading) -> Self {
        use crate::battery::{Kind, Power, Value};
        Self {
            value: Value::reported(value.percent),
            kind: Kind::Mouse,
            power: match value.charge_code {
                0 => Power::Battery,
                1 => Power::Charging,
                2 => Power::Full,
                _ => Power::Unknown,
            },
            connected: value.linked(),
        }
    }
}

pub fn request() -> [u8; 64] {
    let mut result = [0; 64];
    result[..9].copy_from_slice(&[0x4D, 1, 1, 0, 0, 9, 0, 0, 8]);
    result
}

pub fn parse(frame: &[u8]) -> Result<Option<Reading>, ParseError> {
    if frame.len() < 9 || frame[0] != 0x4D || frame[4..6] != [0, 9] {
        return Ok(None);
    }
    if frame[1] != 1 || frame[6] != 0 || frame[7] != 0 {
        return Err("电量应答版本或事务字段不匹配".into());
    }
    let size = frame[3] as usize;
    if 8 + size > frame.len() {
        return Err("电量应答长度无效".into());
    }
    // This receiver emits a one-byte FF negative acknowledgement without a CRC,
    // even though its flags request CRC for normal data. It is never a Reading.
    if size == 1 && frame[8] == 0xFF {
        return Err(ParseError::Busy);
    }
    if frame[2] & 1 != 0 {
        if 8 + size >= frame.len() {
            return Err("电量应答缺少校验".into());
        }
        let xor = frame[2..8 + size].iter().fold(0, |sum, byte| sum ^ byte);
        if xor != frame[8 + size] {
            return Err("电量应答校验失败".into());
        }
    }
    if size < 14 || frame[8..10] != [0x37, 0x38] || frame[20] > 100 {
        return Err("电量应答身份或数值无效".into());
    }
    // The actual A7 V3 Pro reports 0x4026; this field is not the shell model ID.
    Ok(Some(Reading {
        percent: frame[20],
        charge_code: frame[19],
        connection_code: frame[18],
        product_field: u16::from_le_bytes([frame[10], frame[11]]),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captured_a7_usb_charging_is_connected_and_not_a_low_battery_alert() {
        let bytes = [
            0x4d, 1, 1, 0x10, 0, 9, 0, 0, 0x37, 0x38, 0x26, 0x40, 4, 0, 0, 0, 0, 0x10, 1, 1, 0x2e,
            0, 8, 0xe4, 0xa7,
        ];
        let mut frame = [0; 64];
        frame[..bytes.len()].copy_from_slice(&bytes);
        let reading = parse(&frame).unwrap().unwrap();
        assert_eq!(reading.percent, 46);
        assert_eq!(reading.connection_code, 1);
        assert!(reading.linked());
        let reading: crate::battery::Reading = reading.into();
        assert_eq!(reading.power, crate::battery::Power::Charging);
        assert!(!reading.discharging());
    }
    fn capture() -> [u8; 64] {
        let mut frame = [0; 64];
        frame[..25].copy_from_slice(&[
            0x4D, 1, 1, 16, 0, 9, 0, 0, 0x37, 0x38, 0x26, 0x40, 4, 0, 0, 0, 0, 0x10, 2, 0, 0x2B, 0,
            8, 0xE4, 0xA0,
        ]);
        frame
    }
    #[test]
    fn real_a7_v3_pro_capture() {
        assert_eq!(
            parse(&capture()).unwrap().unwrap(),
            Reading {
                percent: 43,
                charge_code: 0,
                connection_code: 2,
                product_field: 0x4026
            }
        );
    }
    #[test]
    fn rejects_corruption_and_truncation() {
        let mut frame = capture();
        frame[20] = 101;
        assert!(parse(&frame).is_err());
        assert!(parse(&capture()[..22]).is_err());
    }
    #[test]
    fn busy_negative_acknowledgement_has_no_normal_data_checksum() {
        let frame = [0x4D, 1, 1, 1, 0, 9, 0, 0, 0xFF, 0];
        assert_eq!(parse(&frame), Err(ParseError::Busy));
        let mut wrong_transaction = frame;
        wrong_transaction[6] = 1;
        assert!(matches!(
            parse(&wrong_transaction),
            Err(ParseError::Invalid(_))
        ));
    }
    #[test]
    fn data_checksum_is_still_strict() {
        let mut frame = capture();
        frame[24] ^= 1;
        assert_eq!(parse(&frame), Err(ParseError::Invalid("电量应答校验失败")));
    }
    #[test]
    fn unrelated_input_is_not_battery() {
        let mut f = capture();
        f[5] = 1;
        assert_eq!(parse(&f).unwrap(), None);
    }
    #[test]
    fn disconnected_zero_never_means_flat_battery_alert() {
        let r = Reading {
            percent: 0,
            charge_code: 0,
            connection_code: 0,
            product_field: 0x4026,
        };
        assert!(!crate::battery::Reading::from(r).discharging());
    }
}
