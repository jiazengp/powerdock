//! Minimal HID++ transport over Windows-owned asynchronous HID reports.
//!
//! Only preselected Logitech FF00 short/long status collections are opened.
//! No feature writes, guessed report IDs, standard input capture, or native
//! stack-backed OVERLAPPED buffers are used. Cancel is a request to Windows;
//! timeout retires the whole channel so late completion cannot be reused.
mod open;
mod route;
use crate::adapters::device;
pub use open::open;
use route::Route;

use std::{
    error::Error,
    future::{Future, IntoFuture},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use ::hidpp::{async_trait, channel::RawHidChannel};
use async_channel::{Receiver, Sender, TrySendError};
use futures::{FutureExt, pin_mut, select_biased};
use windows::{Devices::HumanInterfaceDevice::HidDevice, Storage::Streams::DataWriter};
use windows_future::IAsyncOperation;

const WRITE_FOR: Duration = Duration::from_millis(750);
const CHECK_EVERY: Duration = Duration::from_millis(20);
const MAX_PENDING_REPORTS: usize = 64;

type BoxError = Box<dyn Error + Send + Sync>;

struct State {
    generation: Arc<AtomicU64>,
    expected: u64,
    retired: AtomicBool,
    sender: Sender<Vec<u8>>,
    receiver: Receiver<Vec<u8>>,
}
impl State {
    fn new(generation: Arc<AtomicU64>, expected: u64) -> Arc<Self> {
        let (sender, receiver) = async_channel::bounded(MAX_PENDING_REPORTS);
        Arc::new(Self {
            generation,
            expected,
            retired: AtomicBool::new(false),
            sender,
            receiver,
        })
    }
    fn active(&self) -> bool {
        !self.retired.load(Ordering::Acquire)
            && self.generation.load(Ordering::Acquire) == self.expected
    }
    fn retire(&self) {
        self.retired.store(true, Ordering::Release);
        self.sender.close();
    }
    fn accept(&self, route: Route, data: Vec<u8>) -> bool {
        if !self.active() || !route.accepts(&data) {
            return false;
        }
        match self.sender.try_send(data) {
            Ok(()) => true,
            // Losing a status/reply frame can violate request ordering. Retire
            // instead of silently replacing a previous reply in a full queue.
            Err(TrySendError::Full(_) | TrySendError::Closed(_)) => {
                self.retire();
                false
            }
        }
    }
    async fn read(&self, destination: &mut [u8]) -> Result<usize, BoxError> {
        if !self.active() {
            return std::future::pending().await;
        }
        let Ok(packet) = self.receiver.recv().await else {
            self.retire();
            // RawHidChannel read errors are retried by the upstream reader.
            // A permanent failure must park until its close-select cancels us.
            return std::future::pending().await;
        };
        if !self.active() {
            return std::future::pending().await;
        }
        let Some(output) = destination.get_mut(..packet.len()) else {
            self.retire();
            return Err(message("invalid_reply: HID++ input buffer too short"));
        };
        output.copy_from_slice(&packet);
        Ok(packet.len())
    }
}

struct Endpoint {
    device: HidDevice,
    token: Option<i64>,
    route: Route,
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        if let Some(token) = self.token.take() {
            let _ = self.device.RemoveInputReportReceived(token);
        }
        let _ = self.device.Close();
    }
}

pub struct Transport {
    endpoints: Vec<Endpoint>,
    state: Arc<State>,
    vendor_id: u16,
    product_id: u16,
}
impl Transport {
    pub fn retire(&self) {
        self.state.retire();
    }
}
impl Drop for Transport {
    fn drop(&mut self) {
        self.retire();
    }
}

fn message(text: &str) -> BoxError {
    Box::new(io::Error::other(text.to_owned()))
}
fn win_error(error: windows::core::Error) -> String {
    match error.code().0 as u32 {
        0x80070005 => "access_denied: HID++ collection access denied",
        0x80070020 => "busy: HID++ collection is exclusively open",
        0x80070002 | 0x8007048f | 0x80070490 => "disconnected: HID++ collection unavailable",
        _ => "invalid_reply: Windows HID++ operation failed",
    }
    .into()
}

// A dropped write is a channel boundary, including a library transaction that
// times out while SendOutputReportAsync is pending. Windows retains the report
// and its buffer; no native buffer is freed on an unconfirmed CancelIoEx.
struct Lease {
    state: Arc<State>,
    armed: bool,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if self.armed {
            self.state.retire();
        }
    }
}
struct WriteGuard {
    operation: IAsyncOperation<u32>,
    lease: Lease,
}
impl Drop for WriteGuard {
    fn drop(&mut self) {
        if self.lease.armed {
            let _ = self.operation.Cancel();
        }
    }
}

async fn until<T>(
    operation: impl Future<Output = Result<T, String>>,
    state: &State,
    deadline: Instant,
) -> Result<T, String> {
    let operation = operation.fuse();
    pin_mut!(operation);
    loop {
        if !state.active() {
            return Err("cancelled: HID++ transport retired".into());
        }
        if Instant::now() >= deadline {
            return Err("timeout: HID++ output report deadline".into());
        }
        let check = futures_timer::Delay::new(
            CHECK_EVERY.min(deadline.saturating_duration_since(Instant::now())),
        )
        .fuse();
        pin_mut!(check);
        select_biased! {
            result = operation => {
                if !state.active() { return Err("cancelled: HID++ late output completion".into()); }
                if Instant::now() >= deadline { return Err("timeout: HID++ late output completion".into()); }
                return result;
            },
            _ = check => {},
        }
    }
}

#[async_trait]
impl RawHidChannel for Transport {
    fn vendor_id(&self) -> u16 {
        self.vendor_id
    }
    fn product_id(&self) -> u16 {
        self.product_id
    }
    fn supports_short_long_hidpp(&self) -> Option<(bool, bool)> {
        Some((
            self.endpoints.iter().any(|e| e.route == Route::Short),
            self.endpoints.iter().any(|e| e.route == Route::Long),
        ))
    }
    fn is_connected(&self) -> bool {
        self.state.active()
    }
    async fn read_report(&self, buf: &mut [u8]) -> Result<usize, BoxError> {
        self.state.read(buf).await
    }
    async fn write_report(&self, src: &[u8]) -> Result<usize, BoxError> {
        if !self.state.active() {
            return Err(message("disconnected: HID++ transport retired"));
        }
        let endpoint = self
            .endpoints
            .iter()
            .find(|e| e.route.accepts(src))
            .ok_or_else(|| message("unsupported: HID++ report ID/length not declared"))?;
        let report = endpoint
            .device
            .CreateOutputReportById(u16::from(endpoint.route.id()))
            .map_err(|e| message(&win_error(e)))?;
        let writer = DataWriter::new().map_err(|e| message(&win_error(e)))?;
        let written = writer
            .WriteBytes(src)
            .and_then(|()| writer.DetachBuffer())
            .and_then(|buffer| report.SetData(&buffer));
        let _ = writer.Close();
        written.map_err(|e| message(&win_error(e)))?;
        let operation = endpoint
            .device
            .SendOutputReportAsync(&report)
            .map_err(|e| message(&win_error(e)))?;
        let mut guard = WriteGuard {
            operation: operation.clone(),
            lease: Lease {
                state: self.state.clone(),
                armed: true,
            },
        };
        let count = until(
            async move { operation.into_future().await.map_err(win_error) },
            &self.state,
            Instant::now() + WRITE_FOR,
        )
        .await
        .map_err(|e| message(&e))?;
        if count as usize != src.len() {
            return Err(message("invalid_reply: incomplete HID++ output report"));
        }
        guard.lease.armed = false;
        Ok(count as usize)
    }
    async fn get_report_descriptor(&self, _: &mut [u8]) -> Result<usize, BoxError> {
        Err(message(
            "unsupported: HID++ descriptor must be verified before channel construction",
        ))
    }
}

#[cfg(test)]
mod tests;
