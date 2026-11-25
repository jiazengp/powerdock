//! 41E4:2001 read-only battery interoperability. No settings opcodes.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Status {
    pub percent: u8,
    pub flags: u8,
}
pub const WAITING: &str = "键盘暂未响应，可能休眠，按键后刷新";
// The vendor/MIT capture uses a 19-byte command; hidapi pads it to the 20-byte Windows report.
pub fn vendor_request() -> [u8; 20] {
    let mut packet = [0; 20];
    packet[..6].copy_from_slice(&[0x13, 0x4a, 1, 0, 0, 2]);
    packet
}
pub fn request() -> [u8; 20] {
    let mut packet = [0; 20];
    packet[..3].copy_from_slice(&[0x13, 0x4a, 1]);
    packet[19] = 0x5e;
    packet
}
pub fn parse(bytes: &[u8]) -> Result<Option<Status>, &'static str> {
    if bytes.first() != Some(&0x13) || bytes.get(1) != Some(&0x4a) {
        return Ok(None);
    }
    if bytes.len() != 20 || bytes[2..5] != [1, 0, 2] {
        return Err("键盘电量应答格式不符");
    }
    if bytes[..19].iter().fold(0u8, |sum, b| sum.wrapping_add(*b)) != bytes[19] {
        return Err("键盘电量校验失败");
    }
    if bytes[5] > 100 {
        return Err("键盘返回未知电量");
    }
    Ok(Some(Status {
        percent: bytes[5],
        flags: bytes[6],
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captured_full_keyboard_and_corruption() {
        let mut reply = [
            0x13, 0x4a, 1, 0, 2, 100, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xc5,
        ];
        assert_eq!(parse(&reply).unwrap().unwrap().percent, 100);
        assert_eq!(parse(&reply).unwrap().unwrap().flags, 1);
        reply[5] = 99;
        assert!(parse(&reply).is_err());
        assert!(parse(&reply[..7]).is_err());
    }
    #[test]
    fn zero_is_valid_and_unknown_is_rejected() {
        let mut reply = [
            0x13, 0x4a, 1, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        reply[19] = reply[..19].iter().fold(0u8, |s, b| s.wrapping_add(*b));
        assert_eq!(parse(&reply).unwrap().unwrap().percent, 0);
        reply[5] = 101;
        reply[19] = reply[..19].iter().fold(0u8, |s, b| s.wrapping_add(*b));
        assert!(parse(&reply).is_err());
        let cmd = request();
        assert_eq!(
            cmd[19],
            cmd[..19].iter().fold(0u8, |s, b| s.wrapping_add(*b))
        );
    }
}
