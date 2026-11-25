//! Sennheiser GAIA3 read-only product/capability and battery interoperability. Native nonblocking WinSock; no pairing or settings.
mod socket;

pub(super) fn gaia_service_node(instance: &str) -> bool {
    instance
        .to_ascii_lowercase()
        .starts_with("bthenum\\{a2129ff3-081b-4c45-8afe-469d9c4842ec}_")
}
use crate::battery::{Component, Kind, Power, Reading, Value};
use sennheiser_protocol::{
    Capabilities, Packet, Product, Update, Version,
    ids::{RESPONSE_BIT, VENDOR_SENNHEISER, sn},
};
use socket::{Socket, Winsock, active, net_error};
use std::{
    mem::{size_of, zeroed},
    ptr::null_mut,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use windows_sys::{
    Win32::{
        Devices::Bluetooth::{AF_BTH, BTHPROTO_RFCOMM, SOCKADDR_BTH},
        Networking::WinSock::*,
    },
    core::GUID,
};
const SERVICE: GUID = GUID::from_u128(0xa2129ff3_081b_4c45_8afe_469d9c4842ec);
fn request(command: u16) -> Result<Vec<u8>, String> {
    if !matches!(
        command,
        sn::MODEL_ID_GET
            | sn::SYSTEM_RELEASE_VERSION_GET
            | sn::BATTERY_LEVEL_GET
            | sn::CHARGING_STATE_GET
    ) {
        return Err("unsupported: command is not a read-only battery/identity GET".into());
    }
    Packet::new(VENDOR_SENNHEISER, command, vec![])
        .encode()
        .map_err(|e| e.to_string())
}
#[derive(Clone, Debug)]
pub struct Identity {
    address: u64,
    product: Product,
    firmware: Version,
    learned: Instant,
}
pub struct Observation {
    pub reading: Reading,
    pub components: Vec<Component>,
}
fn updates(command: u16, payload: &[u8]) -> Vec<Update> {
    sennheiser_protocol::decode(
        &Packet::new(VENDOR_SENNHEISER, command | RESPONSE_BIT, payload),
        None,
    )
}
fn identity(address: u64, model: &[u8], firmware: &[u8]) -> Result<Identity, String> {
    if model.is_empty() || model.len() > 64 || firmware.len() != 6 {
        return Err("unsupported: invalid Sennheiser identity payload".into());
    }
    let model_end = model.iter().position(|b| *b == 0).unwrap_or(model.len());
    if model[model_end..].iter().any(|b| *b != 0)
        || std::str::from_utf8(&model[..model_end]).is_err()
    {
        return Err("unsupported: invalid Sennheiser ModelId".into());
    }
    let model = updates(sn::MODEL_ID_GET, model)
        .into_iter()
        .find_map(|u| {
            if let Update::ModelId(m) = u {
                Some(m)
            } else {
                None
            }
        })
        .ok_or("unsupported: missing ModelId")?;
    let product = Product::from_model_id(&model)
        .map(|(p, _)| p)
        .ok_or("unsupported: unknown Sennheiser ModelId")?;
    let firmware = updates(sn::SYSTEM_RELEASE_VERSION_GET, firmware)
        .into_iter()
        .find_map(|u| {
            if let Update::FirmwareVersion(v) = u {
                Some(v)
            } else {
                None
            }
        })
        .ok_or("unsupported: missing firmware")?;
    Ok(Identity {
        address,
        product,
        firmware,
        learned: Instant::now(),
    })
}
impl Identity {
    fn permitted(&self, allow_family: bool) -> Result<(), String> {
        if self.product != Product::Momentum4 && !allow_family {
            return Err("unsupported: Sennheiser family experimental profile disabled".into());
        }
        if !Capabilities::new(self.product, self.firmware).rfcomm_control {
            return Err("unsupported: Sennheiser firmware lacks RFCOMM control".into());
        }
        Ok(())
    }
}
fn battery_observation(
    product: Product,
    payload: &[u8],
    power: Power,
) -> Result<Observation, String> {
    let earbuds = product.is_earbuds();
    if (!earbuds && payload.len() != 1) || (earbuds && !(2..=3).contains(&payload.len())) {
        return Err("unsupported: unexpected product battery layout".into());
    }
    let decoded = updates(sn::BATTERY_LEVEL_GET, payload);
    let percent = decoded
        .iter()
        .find_map(|u| {
            if let Update::Battery(v) = u {
                Some(*v)
            } else {
                None
            }
        })
        .flatten();
    let mut components = Vec::new();
    if earbuds {
        for update in decoded {
            if let Update::Earbuds(b) = update {
                for (role, v) in [("left", b.left), ("right", b.right), ("case", b.case)] {
                    // A two-bud reply does not assert that a case component exists.
                    if role == "case" && payload.len() < 3 {
                        continue;
                    }
                    components.push(Component {
                        id: format!("sennheiser-{role}"),
                        role,
                        value: v.map_or(Value::Unknown, Value::reported),
                        power: Power::Unknown,
                        external_power: None,
                        identifier: None,
                        raw_power_state: None,
                    });
                }
            }
        }
    }
    Ok(Observation {
        reading: Reading {
            value: percent.map_or(Value::Unknown, Value::reported),
            kind: Kind::Headset,
            power: if earbuds { Power::Unknown } else { power },
            connected: true,
        },
        components,
    })
}
#[derive(Default)]
struct Decoder {
    buffer: Vec<u8>,
}
impl Decoder {
    fn push(&mut self, bytes: &[u8]) -> Result<(), String> {
        if self.buffer.len() + bytes.len() > 8192 {
            return Err("耳机状态流超过限制".into());
        }
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }
    fn next(&mut self) -> Result<Option<(u16, Vec<u8>)>, String> {
        loop {
            let Some(start) = self.buffer.iter().position(|b| *b == 0xff) else {
                self.buffer.clear();
                return Ok(None);
            };
            if start > 0 {
                self.buffer.drain(..start);
            }
            if self.buffer.len() < 4 {
                return Ok(None);
            }
            if ![1, 3, 4].contains(&self.buffer[1]) || self.buffer[2] & !3 != 0 {
                return Err("耳机状态帧版本不符".into());
            }
            let flags = self.buffer[2];
            let header = if flags & 2 != 0 { 5 } else { 4 };
            if self.buffer.len() < header {
                return Ok(None);
            }
            let length = if header == 5 {
                u16::from_be_bytes([self.buffer[3], self.buffer[4]]) as usize
            } else {
                self.buffer[3] as usize
            };
            if length > 4096 {
                return Err("耳机状态负载超过限制".into());
            }
            let end = header + 4 + length;
            let total = end + usize::from(flags & 1 != 0);
            if self.buffer.len() < total {
                return Ok(None);
            }
            if flags & 1 != 0 && self.buffer[..total].iter().fold(0, |sum, b| sum ^ b) != 0 {
                return Err("耳机状态帧校验失败".into());
            }
            let vendor = u16::from_be_bytes([self.buffer[header], self.buffer[header + 1]]);
            let command = u16::from_be_bytes([self.buffer[header + 2], self.buffer[header + 3]]);
            let payload = self.buffer[header + 4..end].to_vec();
            self.buffer.drain(..total);
            if vendor == 0x0495 {
                return Ok(Some((command, payload)));
            }
        }
    }
}
fn query(
    socket: &Socket,
    decoder: &mut Decoder,
    command: u16,
    deadline: Instant,
    generation: &AtomicU64,
    expected: u64,
) -> Result<Vec<u8>, String> {
    socket.write(&request(command)?, deadline, generation, expected)?;
    let mut bytes = [0; 512];
    let mut received = 0;
    loop {
        while let Some((opcode, payload)) = decoder.next()? {
            if opcode == (command | RESPONSE_BIT) {
                return Ok(payload);
            }
            if opcode == (command | 0x0180) {
                return Err(format!("耳机拒绝只读查询 0x{command:02X}"));
            }
        }
        let n = socket.read(&mut bytes, deadline, generation, expected)?;
        received += n;
        if received > 8192 {
            return Err("耳机无匹配电量应答".into());
        }
        decoder.push(&bytes[..n])?;
    }
}
fn charging(bytes: &[u8]) -> Power {
    if !(1..=2).contains(&bytes.len()) || bytes.get(1).is_some_and(|v| *v != 0) {
        return Power::Unknown;
    }
    match sennheiser_protocol::ChargingState::from_u8(bytes[0]) {
        Some(sennheiser_protocol::ChargingState::Disconnected) => Power::Battery,
        Some(sennheiser_protocol::ChargingState::Charging) => Power::Charging,
        Some(sennheiser_protocol::ChargingState::Complete) => Power::Full,
        None => Power::Unknown,
    }
}
pub fn read(
    address: u64,
    generation: &AtomicU64,
    expected: u64,
    cached: &mut Option<Identity>,
    allow_family: bool,
) -> Result<Observation, String> {
    if cached
        .as_ref()
        .is_some_and(|i| i.address != address || i.learned.elapsed() > Duration::from_secs(300))
    {
        *cached = None;
    }
    if let Some(i) = cached.as_ref() {
        i.permitted(allow_family)?;
    }
    let deadline = Instant::now() + Duration::from_millis(3000);
    active(deadline, generation, expected)?;
    let mut data: WSADATA = unsafe { zeroed() };
    if unsafe { WSAStartup(0x0202, &mut data) } != 0 {
        return Err(net_error("无法初始化蓝牙控制通道"));
    }
    let _winsock = Winsock;
    let raw = unsafe { socket(AF_BTH as i32, SOCK_STREAM, BTHPROTO_RFCOMM as i32) };
    if raw == INVALID_SOCKET {
        return Err(net_error("无法创建耳机控制通道"));
    }
    let socket = Socket(raw);
    let mut nonblocking = 1;
    if unsafe { ioctlsocket(raw, FIONBIO, &mut nonblocking) } == SOCKET_ERROR {
        return Err(net_error("设置耳机非阻塞模式失败"));
    }
    let target = SOCKADDR_BTH {
        addressFamily: AF_BTH,
        btAddr: address,
        serviceClassId: SERVICE,
        port: 0,
    };
    let connected = unsafe {
        connect(
            raw,
            (&target as *const SOCKADDR_BTH).cast(),
            size_of::<SOCKADDR_BTH>() as i32,
        )
    };
    if connected == SOCKET_ERROR {
        if ![WSAEWOULDBLOCK, WSAEINPROGRESS, WSAEALREADY].contains(&unsafe { WSAGetLastError() }) {
            return Err(net_error("耳机 GAIA 服务暂不可用"));
        }
        socket.wait(true, deadline, generation, expected)?;
        let mut error = 0i32;
        let mut size = size_of::<i32>() as i32;
        if unsafe {
            getsockopt(
                raw,
                SOL_SOCKET,
                SO_ERROR,
                (&mut error as *mut i32).cast(),
                &mut size,
            )
        } == SOCKET_ERROR
            || error != 0
        {
            return Err(format!("耳机 GAIA 连接失败：{error}"));
        }
    }
    let mut decoder = Decoder::default();
    if cached.is_none() {
        let model = query(
            &socket,
            &mut decoder,
            sn::MODEL_ID_GET,
            deadline.min(Instant::now() + Duration::from_millis(600)),
            generation,
            expected,
        )?;
        let firmware = query(
            &socket,
            &mut decoder,
            sn::SYSTEM_RELEASE_VERSION_GET,
            deadline.min(Instant::now() + Duration::from_millis(600)),
            generation,
            expected,
        )?;
        let found = identity(address, &model, &firmware)?;
        crate::diagnostics::record(
            "provider_identified",
            serde_json::json!({
            "provider":"sennheiser_gaia","product":found.product.name(),
            "firmware":found.firmware.to_string(),"hardware_verified_here":false}),
        );
        *cached = Some(found);
    }
    let identified = cached
        .as_ref()
        .ok_or("unsupported: no Sennheiser identity")?;
    identified.permitted(allow_family)?;
    let payload = query(
        &socket,
        &mut decoder,
        sn::BATTERY_LEVEL_GET,
        deadline,
        generation,
        expected,
    )?;
    // TWS charging/parent power semantics stay unknown; don't reuse over-ear layout.
    let power = if identified.product.is_earbuds() {
        Power::Unknown
    } else {
        query(
            &socket,
            &mut decoder,
            sn::CHARGING_STATE_GET,
            deadline,
            generation,
            expected,
        )
        .map_or(Power::Unknown, |v| charging(&v))
    };
    // Optional charging can consume the remaining deadline without invalidating
    // an already received battery reply. Epoch cancellation still rejects it.
    if generation.load(Ordering::Acquire) != expected {
        return Err("设备会话已变更".into());
    }
    battery_observation(identified.product, &payload, power)
}

#[cfg(test)]
mod tests;
