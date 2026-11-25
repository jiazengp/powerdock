use super::*;
use futures::{executor::block_on, task::noop_waker_ref};
use std::{
    pin::pin,
    task::{Context, Poll},
};

fn state() -> Arc<State> {
    State::new(Arc::new(AtomicU64::new(1)), 1)
}
fn packet(route: Route, marker: u8) -> Vec<u8> {
    let mut bytes = vec![0; route.len()];
    bytes[0] = route.id();
    bytes[1] = marker;
    bytes
}
fn metadata(usage: u16, bytes: u16) -> device::Device {
    device::Device {
        path: Vec::new(),
        container: Some("physical".into()),
        vendor_id: 0x046d,
        product_id: 0xc52b,
        product: "fixture".into(),
        usage_page: 0xff00,
        usage,
        input_bytes: bytes,
        output_bytes: bytes,
    }
}
#[test]
fn matching_requires_verified_vendor_collection_and_exact_short_long_lengths() {
    assert_eq!(Route::from_metadata(&metadata(1, 7)), Some(Route::Short));
    assert_eq!(Route::from_metadata(&metadata(2, 20)), Some(Route::Long));
    let mut invalid = metadata(1, 7);
    invalid.vendor_id = 0x1234;
    assert_eq!(Route::from_metadata(&invalid), None);
    invalid = metadata(1, 7);
    invalid.usage_page = 1;
    assert_eq!(Route::from_metadata(&invalid), None);
    assert_eq!(Route::from_metadata(&metadata(1, 20)), None);
    assert_eq!(Route::from_metadata(&metadata(3, 20)), None);
    assert!(
        win_error(windows::core::Error::from_hresult(windows::core::HRESULT(
            0x80070005u32 as i32
        )))
        .starts_with("access_denied:")
    );
    assert!(
        win_error(windows::core::Error::from_hresult(windows::core::HRESULT(
            0x80070020u32 as i32
        )))
        .starts_with("busy:")
    );
    assert!(!Route::Short.accepts(&packet(Route::Long, 1)));
    assert!(!Route::Long.accepts(&packet(Route::Short, 1)));
}
#[test]
fn bad_metadata_and_ambiguous_routes_never_open_hardware() {
    let generation = Arc::new(AtomicU64::new(1));
    for endpoints in [
        vec![],
        vec![metadata(3, 20)],
        vec![metadata(1, 7), metadata(1, 7)],
    ] {
        assert!(open(&endpoints, generation.clone(), 1).is_err());
    }
    let mut separate = metadata(2, 20);
    separate.container = Some("another receiver".into());
    assert!(open(&[metadata(1, 7), separate], generation, 1).is_err());
}
#[test]
fn retired_and_previous_generation_callbacks_cannot_supply_a_late_reply() {
    let current = state();
    assert!(current.accept(Route::Short, packet(Route::Short, 1)));
    current.generation.fetch_add(1, Ordering::AcqRel);
    assert!(!current.accept(Route::Short, packet(Route::Short, 2)));
    let mut buf = [0; 20];
    let mut read = pin!(current.read(&mut buf));
    assert!(matches!(
        read.as_mut()
            .poll(&mut Context::from_waker(noop_waker_ref())),
        Poll::Pending
    ));
    current.retire();
    assert!(!current.accept(Route::Short, packet(Route::Short, 3)));
}
#[test]
fn queue_overflow_is_bounded_and_retires_instead_of_dropping_a_reply() {
    let current = state();
    for _ in 0..MAX_PENDING_REPORTS {
        assert!(current.accept(Route::Short, packet(Route::Short, 1)));
    }
    assert!(!current.accept(Route::Short, packet(Route::Short, 2)));
    assert_eq!(current.receiver.len(), MAX_PENDING_REPORTS);
    assert!(!current.active());
}
#[test]
fn cancelled_read_future_is_reusable_and_closed_read_parks_without_busy_errors() {
    let current = state();
    let mut scratch = [0; 20];
    {
        let mut read = pin!(current.read(&mut scratch));
        assert!(matches!(
            read.as_mut()
                .poll(&mut Context::from_waker(noop_waker_ref())),
            Poll::Pending
        ));
    }
    assert!(current.accept(Route::Long, packet(Route::Long, 42)));
    assert_eq!(block_on(current.read(&mut scratch)).unwrap(), 20);
    assert_eq!(scratch[1], 42);
    current.retire();
    for _ in 0..8 {
        let mut read = pin!(current.read(&mut scratch));
        assert!(matches!(
            read.as_mut()
                .poll(&mut Context::from_waker(noop_waker_ref())),
            Poll::Pending
        ));
    }
}
#[test]
fn output_deadline_cancel_and_drop_retire_the_lease() {
    let current = state();
    let result = block_on(until(
        std::future::pending::<Result<(), String>>(),
        &current,
        Instant::now() + Duration::from_millis(5),
    ));
    assert!(result.unwrap_err().starts_with("timeout:"));
    {
        let _lease = Lease {
            state: current.clone(),
            armed: true,
        };
    }
    assert!(!current.active());
    assert!(!current.accept(Route::Short, packet(Route::Short, 1)));
    let current = state();
    current.generation.store(2, Ordering::Release);
    assert!(
        block_on(until(
            async { Ok(()) },
            &current,
            Instant::now() + Duration::from_secs(1)
        ))
        .unwrap_err()
        .starts_with("cancelled:")
    );
    let current = state();
    {
        let _completed = Lease {
            state: current.clone(),
            armed: false,
        };
    }
    assert!(current.active());
}
struct Fixture {
    state: Arc<State>,
    reads: Arc<std::sync::atomic::AtomicUsize>,
}
#[async_trait]
impl RawHidChannel for Fixture {
    fn vendor_id(&self) -> u16 {
        0x046d
    }
    fn product_id(&self) -> u16 {
        0xffff
    }
    fn supports_short_long_hidpp(&self) -> Option<(bool, bool)> {
        Some((true, true))
    }
    fn is_connected(&self) -> bool {
        self.state.active()
    }
    async fn read_report(&self, destination: &mut [u8]) -> Result<usize, BoxError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.state.read(destination).await
    }
    async fn write_report(&self, src: &[u8]) -> Result<usize, BoxError> {
        Ok(src.len())
    }
    async fn get_report_descriptor(&self, _: &mut [u8]) -> Result<usize, BoxError> {
        panic!("declared fixture skips descriptor")
    }
}
#[test]
fn upstream_channel_drop_cancels_a_parked_permanent_failure_without_busy_spin() {
    // Run under the controller's external process deadline: an assertion
    // after Drop alone cannot establish that a stuck join would terminate.
    block_on(async {
        let current = state();
        let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let channel = ::hidpp::channel::HidppChannel::from_raw_channel(Fixture {
            state: current.clone(),
            reads: reads.clone(),
        })
        .await
        .unwrap();
        current.retire();
        futures_timer::Delay::new(Duration::from_millis(5)).await;
        assert!(
            reads.load(Ordering::Acquire) <= 1,
            "permanent failure must park, not repeat Err"
        );
        drop(channel);
    });
}
#[test]
fn retiring_an_abandoned_channel_rejects_its_late_reply_and_new_lease_is_independent() {
    block_on(async {
        use ::hidpp::channel::{ChannelError, HidppChannel, HidppMessage};
        let retired = state();
        let channel = HidppChannel::from_raw_channel(Fixture {
            state: retired.clone(),
            reads: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        })
        .await
        .unwrap();
        let message = HidppMessage::Short([2, 7, 0x11, 1, 0, 0]);
        assert!(matches!(
            channel
                .send_with_timeout(message, |_| true, Duration::from_millis(5))
                .await,
            Err(ChannelError::Timeout)
        ));
        retired.retire();
        assert!(!retired.accept(Route::Short, packet(Route::Short, 0xa1)));
        drop(channel);
        let next = state();
        assert!(next.accept(Route::Short, packet(Route::Short, 0xb2)));
        let mut buf = [0; 20];
        assert_eq!(next.read(&mut buf).await.unwrap(), 7);
        assert_eq!(buf[1], 0xb2);
    });
}
#[test]
fn oversized_input_is_not_split_across_subsequent_reads() {
    let current = state();
    assert!(current.accept(Route::Long, packet(Route::Long, 42)));
    assert!(block_on(current.read(&mut [0; 7])).is_err());
    assert!(!current.active());
}
