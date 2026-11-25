//! Own each native I/O buffer until completion, including cancellation drain.
use super::*;
use std::time::{Duration, Instant};

#[cfg(test)]
mod tests;

pub(in crate::adapters) struct Reply<'a, F> {
    read: Pending<'a>,
    deadline: Instant,
    check: F,
}

impl<'a, F: Fn() -> Result<(), String>> Reply<'a, F> {
    pub(in crate::adapters) fn request(
        handle: &'a Handle,
        input: Vec<u8>,
        output: Vec<u8>,
        deadline: Instant,
        check: F,
    ) -> Result<(Self, usize), String> {
        check()?;
        let read = Pending::begin(handle, input, false)?;
        check()?;
        let mut write = Pending::begin(handle, output, true)?;
        let written = write.wait(deadline, &check)?;
        Ok((
            Self {
                read,
                deadline,
                check,
            },
            written,
        ))
    }

    pub(in crate::adapters) fn next_frame(&mut self) -> Result<&[u8], String> {
        if !self.read.active {
            (self.check)()?;
            self.read.buffer.fill(0);
            self.read.submit(false)?;
        }
        let size = self.read.wait(self.deadline, &self.check)?;
        self.read
            .buffer
            .get(..size)
            .ok_or_else(|| "invalid_reply: HID input size".into())
    }
}

struct Pending<'a> {
    handle: &'a Handle,
    overlap: Box<OVERLAPPED>,
    event: Handle,
    buffer: Vec<u8>,
    active: bool,
}
impl<'a> Pending<'a> {
    fn begin(handle: &'a Handle, buffer: Vec<u8>, write: bool) -> Result<Self, String> {
        let event = Handle(unsafe { CreateEventW(null(), 1, 0, null()) });
        if event.0.is_null() {
            return Err(error("创建 I/O 事件失败"));
        }
        let mut value = Self {
            handle,
            overlap: Box::new(unsafe { zeroed() }),
            event,
            buffer,
            active: false,
        };
        value.submit(write)?;
        Ok(value)
    }

    fn submit(&mut self, write: bool) -> Result<(), String> {
        debug_assert!(!self.active);
        unsafe {
            *self.overlap = zeroed();
            self.overlap.hEvent = self.event.0;
            let success = if write {
                WriteFile(
                    self.handle.0,
                    self.buffer.as_ptr(),
                    self.buffer.len() as u32,
                    null_mut(),
                    &mut *self.overlap,
                )
            } else {
                ReadFile(
                    self.handle.0,
                    self.buffer.as_mut_ptr(),
                    self.buffer.len() as u32,
                    null_mut(),
                    &mut *self.overlap,
                )
            };
            if success == 0 && GetLastError() != ERROR_IO_PENDING {
                return Err(error(if write {
                    "发送电量查询失败"
                } else {
                    "启动电量读取失败"
                }));
            }
            self.active = true;
            Ok(())
        }
    }

    fn wait(
        &mut self,
        deadline: Instant,
        check: &impl Fn() -> Result<(), String>,
    ) -> Result<usize, String> {
        unsafe {
            loop {
                check()?;
                let remaining = deadline.saturating_duration_since(Instant::now());
                let wait_ms = remaining.min(Duration::from_millis(50)).as_millis().max(1);
                match WaitForSingleObject(self.event.0, wait_ms as u32) {
                    WAIT_OBJECT_0 => {
                        let mut transferred = 0;
                        let success =
                            GetOverlappedResult(self.handle.0, &*self.overlap, &mut transferred, 0);
                        if success == 0 {
                            // A reported incomplete request still owns its buffer.
                            self.active = GetLastError() == ERROR_IO_INCOMPLETE;
                            return Err(error("读取电量应答失败"));
                        }
                        self.active = false;
                        check()?;
                        return Ok(transferred as usize);
                    }
                    WAIT_TIMEOUT => {}
                    _ => return Err(error("等待 I/O 失败")),
                }
            }
        }
    }
}
impl Drop for Pending<'_> {
    fn drop(&mut self) {
        if self.active {
            unsafe {
                CancelIoEx(self.handle.0, &*self.overlap);
                // CancelIoEx is only a request; kernel-owned buffers must live until completion.
                let mut transferred = 0;
                GetOverlappedResult(self.handle.0, &*self.overlap, &mut transferred, 1);
            }
        }
    }
}
