//! Nonblocking RFCOMM I/O bounded by the caller deadline and generation.
use super::*;

pub(super) struct Winsock;
impl Drop for Winsock {
    fn drop(&mut self) {
        unsafe {
            WSACleanup();
        }
    }
}
pub(super) struct Socket(pub(super) SOCKET);
impl Drop for Socket {
    fn drop(&mut self) {
        unsafe {
            closesocket(self.0);
        }
    }
}
pub(super) fn net_error(context: &str) -> String {
    format!("{context}：WinSock {}", unsafe { WSAGetLastError() })
}
pub(super) fn active(
    deadline: Instant,
    generation: &AtomicU64,
    expected: u64,
) -> Result<(), String> {
    if generation.load(Ordering::Acquire) != expected {
        return Err("设备会话已变更".into());
    }
    if Instant::now() >= deadline {
        return Err("耳机电量查询超时".into());
    }
    Ok(())
}
impl Socket {
    pub(super) fn wait(
        &self,
        write: bool,
        deadline: Instant,
        generation: &AtomicU64,
        expected: u64,
    ) -> Result<(), String> {
        loop {
            active(deadline, generation, expected)?;
            let mut ready: FD_SET = unsafe { zeroed() };
            ready.fd_count = 1;
            ready.fd_array[0] = self.0;
            let mut failed = ready;
            let remaining = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(50));
            let timeout = TIMEVAL {
                tv_sec: 0,
                tv_usec: remaining.as_micros().max(1) as i32,
            };
            let rc = unsafe {
                select(
                    0,
                    if write { null_mut() } else { &mut ready },
                    if write { &mut ready } else { null_mut() },
                    &mut failed,
                    &timeout,
                )
            };
            if rc == SOCKET_ERROR {
                return Err(net_error("等待耳机通道失败"));
            }
            if failed.fd_count > 0 {
                return Err(net_error("耳机通道不可用"));
            }
            if ready.fd_count > 0 {
                return Ok(());
            }
        }
    }
    pub(super) fn write(
        &self,
        bytes: &[u8],
        deadline: Instant,
        generation: &AtomicU64,
        expected: u64,
    ) -> Result<(), String> {
        let mut offset = 0;
        while offset < bytes.len() {
            active(deadline, generation, expected)?;
            let n = unsafe {
                send(
                    self.0,
                    bytes[offset..].as_ptr(),
                    (bytes.len() - offset) as i32,
                    0,
                )
            };
            if n > 0 {
                offset += n as usize;
            } else if n == SOCKET_ERROR && unsafe { WSAGetLastError() } == WSAEWOULDBLOCK {
                self.wait(true, deadline, generation, expected)?;
            } else {
                return Err(net_error("发送耳机只读查询失败"));
            }
        }
        Ok(())
    }
    pub(super) fn read(
        &self,
        buffer: &mut [u8],
        deadline: Instant,
        generation: &AtomicU64,
        expected: u64,
    ) -> Result<usize, String> {
        loop {
            active(deadline, generation, expected)?;
            let n = unsafe { recv(self.0, buffer.as_mut_ptr(), buffer.len() as i32, 0) };
            if n > 0 {
                return Ok(n as usize);
            }
            if n == 0 {
                return Err("耳机已关闭控制通道".into());
            }
            if unsafe { WSAGetLastError() } != WSAEWOULDBLOCK {
                return Err(net_error("读取耳机状态失败"));
            }
            self.wait(false, deadline, generation, expected)?;
        }
    }
}
