//! Shared Windows async ownership, cancellation and deadline contract.
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use windows::core::RuntimeType;
use windows_future::IAsyncOperation;

// WinRT owns an async operation's buffers. Cancellation is requested on timeout;
// the OS may finish an underlying Bluetooth connection later (a Windows API limit).
pub(crate) fn wait<T: RuntimeType>(
    operation: windows::core::Result<IAsyncOperation<T>>,
    deadline: Instant,
    generation: &AtomicU64,
    expected: u64,
) -> Result<T, String> {
    let operation = operation.map_err(|_| "蓝牙接口无法启动查询")?;
    loop {
        if generation.load(Ordering::Acquire) != expected {
            let _ = operation.Cancel();
            return Err("蓝牙查询已取消或超时".into());
        }
        match operation.Status() {
            Ok(status) if status.0 == 0 => {
                if Instant::now() >= deadline {
                    let _ = operation.Cancel();
                    return Err("蓝牙查询已取消或超时".into());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(status) if status.0 == 1 => {
                let result = operation
                    .GetResults()
                    .map_err(|_| "蓝牙查询结果不可用".to_owned())?;
                if generation.load(Ordering::Acquire) != expected || Instant::now() >= deadline {
                    return Err("cancelled: late Bluetooth result".into());
                }
                return Ok(result);
            }
            _ => return Err("蓝牙查询失败或访问未获允许".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_operations_still_obey_epoch_and_deadline() {
        let epoch = AtomicU64::new(2);
        let ready = || Ok(IAsyncOperation::<u32>::ready(Ok(42)));
        assert_eq!(
            wait(ready(), Instant::now() + Duration::from_secs(1), &epoch, 2).unwrap(),
            42
        );
        assert!(wait(ready(), Instant::now() + Duration::from_secs(1), &epoch, 1).is_err());
        assert!(
            wait(
                ready(),
                Instant::now() - Duration::from_millis(1),
                &epoch,
                2
            )
            .is_err()
        );
    }
}
