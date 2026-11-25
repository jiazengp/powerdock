//! Minimal read-only YJX battery exchange, checked against pinned OpenMHub MIT.
//! Native Windows reports include the zero report ID before the 64-byte payload.
use crate::battery::{Kind, Power, Reading, Value};
pub const REPORT_BYTES: usize = 65;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Status,
    Battery,
}
impl Command {
    fn opcode(self) -> u8 {
        match self {
            Self::Status => 0xed,
            Self::Battery => 0x30,
        }
    }
}
pub fn request(command: Command) -> [u8; REPORT_BYTES] {
    let mut frame = [0; REPORT_BYTES];
    match command {
        Command::Status => frame[1..3].copy_from_slice(&[0x55, 0xed]),
        Command::Battery => frame[1..9].copy_from_slice(&[0x55, 0x30, 0xa5, 0x0b, 0x2e, 1, 1, 1]),
    }
    frame
}
fn payload(frame: &[u8], command: Command) -> Result<Option<&[u8]>, String> {
    if frame.first() != Some(&0)
        || frame.get(1) != Some(&0xaa)
        || frame.get(2) != Some(&command.opcode())
    {
        return Ok(None);
    }
    if frame.len() != REPORT_BYTES {
        return Err("YJX 状态报告长度不符".into());
    }
    Ok(Some(&frame[1..]))
}
pub fn matches_reply(frame: &[u8], command: Command) -> Result<bool, String> {
    Ok(payload(frame, command)?.is_some())
}
pub fn read(
    wired: bool,
    mut query: impl FnMut(Command) -> Result<Vec<u8>, String>,
) -> Result<Reading, String> {
    if !wired {
        let frame = query(Command::Status)?;
        let status = payload(&frame, Command::Status)?.ok_or("YJX 接收器状态应答不符")?;
        match status[8] {
            0 => {}
            1 => return Err("鼠标未连接 · 接收器在线不代表外设在线".into()),
            _ => return Err("YJX 返回未知连接状态".into()),
        }
    }
    let frame = query(Command::Battery)?;
    let data = payload(&frame, Command::Battery)?.ok_or("YJX 电量应答不符")?;
    if data[8] > 100 {
        return Err("YJX 返回未知电量".into());
    }
    Ok(Reading {
        value: Value::reported(data[8]),
        kind: Kind::Mouse,
        power: Power::Unknown,
        connected: true,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn reply(command: Command, value: u8) -> Vec<u8> {
        let mut r = vec![0; REPORT_BYTES];
        r[1] = 0xaa;
        r[2] = command.opcode();
        r[9] = value;
        r
    }
    #[test]
    fn windows_report_layout_and_unrelated_frames() {
        let b = request(Command::Battery);
        assert_eq!(&b[..9], &[0, 0x55, 0x30, 0xa5, 0x0b, 0x2e, 1, 1, 1]);
        assert!(b[9..].iter().all(|v| *v == 0));
        assert_eq!(&request(Command::Status)[..3], &[0, 0x55, 0xed]);
        assert!(!matches_reply(&reply(Command::Status, 0), Command::Battery).unwrap());
        let mut r = reply(Command::Battery, 0);
        assert!(matches_reply(&r, Command::Battery).unwrap());
        assert!(matches_reply(&r[..10], Command::Battery).is_err());
        r[0] = 1;
        assert!(!matches_reply(&r, Command::Battery).unwrap());
    }
    #[test]
    fn offline_and_unknown_receiver_never_request_battery() {
        for state in [1, 2, 255] {
            let mut commands = Vec::new();
            assert!(
                read(false, |c| {
                    commands.push(c);
                    Ok(reply(c, state))
                })
                .is_err()
            );
            assert_eq!(commands, vec![Command::Status]);
        }
    }
    #[test]
    fn rf_sequence_wired_sequence_and_zero_unknown() {
        let mut commands = Vec::new();
        let r = read(false, |c| {
            commands.push(c);
            Ok(reply(c, if c == Command::Status { 0 } else { 60 }))
        })
        .unwrap();
        assert_eq!(commands, vec![Command::Status, Command::Battery]);
        assert_eq!(r.value, Value::reported(60));
        assert_eq!(r.power, Power::Unknown);
        let mut commands = Vec::new();
        assert_eq!(
            read(true, |c| {
                commands.push(c);
                Ok(reply(c, 0))
            })
            .unwrap()
            .value,
            Value::reported(0)
        );
        assert_eq!(commands, vec![Command::Battery]);
        for v in [101, 255] {
            assert!(read(true, |c| Ok(reply(c, v))).is_err());
        }
        assert!(read(false, |_| Err("timeout".into())).is_err());
    }
}
