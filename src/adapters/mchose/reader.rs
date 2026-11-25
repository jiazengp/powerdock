use super::{a7, keyboard, profiles, yjx};
use crate::{
    adapters::device::{Device, Handle, QUERY_TIMEOUT, Reply},
    battery::{Kind, Power, Reading, Value},
};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

#[cfg(test)]
mod tests;

fn check_epoch(generation: &AtomicU64, expected: u64) -> Result<(), String> {
    if generation.load(Ordering::Acquire) != expected {
        return Err("设备会话已变更".into());
    }
    Ok(())
}

fn check_transaction(
    generation: &AtomicU64,
    expected: u64,
    deadline: Instant,
) -> Result<(), String> {
    check_epoch(generation, expected)?;
    if Instant::now() >= deadline {
        return Err(QUERY_TIMEOUT.into());
    }
    Ok(())
}

pub fn query(
    device: &Device,
    generation: &AtomicU64,
    expected: u64,
) -> Result<a7::Reading, String> {
    Session::open(device)?.query_a7(generation, expected, false)
}

pub fn query_diagnostic(
    device: &Device,
    generation: &AtomicU64,
    expected: u64,
) -> Result<a7::Reading, String> {
    Session::open(device)?.query_a7(generation, expected, true)
}

pub struct Session {
    handle: Handle,
    protocol: profiles::MouseProtocol,
}

impl Session {
    pub fn open(device: &Device) -> Result<Self, String> {
        let protocol = profiles::mouse_protocol(device).ok_or("拒绝向未经验证的接口发送查询")?;
        let handle = Handle::open_status(device, "无法访问鼠标状态接口")?;
        Ok(Self { handle, protocol })
    }

    pub fn query(&self, generation: &AtomicU64, expected: u64) -> Result<Reading, String> {
        if let profiles::MouseProtocol::Yjx { wired } = self.protocol {
            check_epoch(generation, expected)?;
            self.handle.flush("清理过期鼠标状态失败")?;
            let deadline = Instant::now() + Duration::from_millis(1500);
            return yjx::read(wired, |command| {
                query_yjx_packet(&self.handle, command, generation, expected, deadline)
            });
        }
        self.query_a7(generation, expected, false).map(Into::into)
    }

    fn query_a7(
        &self,
        generation: &AtomicU64,
        expected: u64,
        diagnostic: bool,
    ) -> Result<a7::Reading, String> {
        if !matches!(self.protocol, profiles::MouseProtocol::A7 { .. }) {
            return Err("拒绝通过旧探测入口调用实验协议".into());
        }
        check_epoch(generation, expected)?;
        self.handle.flush("清理过期鼠标状态失败")?;
        let deadline = Instant::now() + Duration::from_millis(3500);
        let mut last_error = QUERY_TIMEOUT.to_owned();
        let mut busy_seen = false;
        for attempt in 0..5 {
            check_epoch(generation, expected)?;
            match query_a7_packet(&self.handle, generation, expected, diagnostic, deadline) {
                Ok(reading) => return Ok(reading),
                Err(error) => {
                    busy_seen |= error == a7::BUSY;
                    last_error = if busy_seen && error == QUERY_TIMEOUT {
                        a7::BUSY.into()
                    } else {
                        error
                    };
                }
            }
            if last_error != a7::BUSY
                || attempt == 4
                || Instant::now() + Duration::from_millis(30) >= deadline
            {
                break;
            }
            if diagnostic {
                eprintln!("Retrying read-only query on the same HID session after: {last_error}");
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        Err(last_error)
    }
}

fn query_yjx_packet(
    handle: &Handle,
    command: yjx::Command,
    generation: &AtomicU64,
    expected: u64,
    deadline: Instant,
) -> Result<Vec<u8>, String> {
    let check = || check_transaction(generation, expected, deadline);
    let (mut reply, _) = Reply::request(
        handle,
        vec![0; yjx::REPORT_BYTES],
        yjx::request(command).to_vec(),
        deadline,
        check,
    )?;
    loop {
        let bytes = reply.next_frame()?;
        if bytes.is_empty() {
            return Err("鼠标状态接口已关闭".into());
        }
        if yjx::matches_reply(bytes, command)? {
            return Ok(bytes.to_vec());
        }
    }
}

fn query_a7_packet(
    handle: &Handle,
    generation: &AtomicU64,
    expected: u64,
    diagnostic: bool,
    deadline: Instant,
) -> Result<a7::Reading, String> {
    let check = || check_transaction(generation, expected, deadline);
    let (mut reply, _) =
        Reply::request(handle, vec![0; 64], a7::request().to_vec(), deadline, check)?;
    loop {
        let bytes = reply.next_frame()?;
        if bytes.is_empty() {
            return Err("鼠标状态接口已关闭".into());
        }
        if diagnostic {
            eprintln!(
                "HID input ({} bytes): {}",
                bytes.len(),
                bytes
                    .iter()
                    .map(|byte| format!("{byte:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
        if let Some(reading) = a7::parse(bytes).map_err(|error| error.to_string())? {
            return Ok(reading);
        }
    }
}

pub fn query_keyboard(
    device: &Device,
    generation: &AtomicU64,
    expected: u64,
) -> Result<Reading, String> {
    if !profiles::mchose_keyboard(device) {
        return Err("键盘接口未经验证".into());
    }
    let handle = Handle::open_status(device, "无法访问键盘电量接口")?;
    handle.flush("清理键盘状态队列失败")?;
    let deadline = Instant::now() + Duration::from_millis(1500);
    let layouts: &[bool] = if profiles::sinowealth_keyboard(device) {
        &[false]
    } else {
        &[true, false, true]
    };
    for &vendor in layouts {
        check_epoch(generation, expected)?;
        let attempt_deadline = deadline.min(Instant::now() + Duration::from_millis(400));
        let check = || check_transaction(generation, expected, attempt_deadline);
        let packet = if vendor {
            keyboard::vendor_request()
        } else {
            keyboard::request()
        };
        let (mut reply, _) = Reply::request(
            &handle,
            vec![0; 20],
            packet.to_vec(),
            attempt_deadline,
            check,
        )?;
        loop {
            let bytes = match reply.next_frame() {
                Ok(bytes) => bytes,
                Err(error) if error == QUERY_TIMEOUT => break,
                Err(error) => return Err(error),
            };
            if bytes.is_empty() {
                return Err("键盘状态接口已关闭".into());
            }
            if let Some(status) = keyboard::parse(bytes).map_err(str::to_owned)? {
                return Ok(Reading {
                    value: Value::reported(status.percent),
                    kind: Kind::Keyboard,
                    power: Power::Unknown,
                    connected: true,
                });
            }
        }
        if Instant::now() >= deadline {
            break;
        }
    }
    Err(keyboard::WAITING.into())
}
