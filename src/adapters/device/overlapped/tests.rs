use super::*;
use std::{
    cell::Cell,
    fs::File,
    io::{Read, Write},
    os::windows::io::FromRawHandle,
    sync::atomic::{AtomicUsize, Ordering},
};
use windows_sys::Win32::System::Pipes::{
    CreateNamedPipeW, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
};

fn pipe() -> (Handle, File) {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
    let name = format!(
        r"\\.\pipe\powerdock-overlapped-test-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    );
    let path = name.encode_utf16().chain([0]).collect::<Vec<_>>();
    let server = Handle(unsafe {
        CreateNamedPipeW(
            path.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
            PIPE_TYPE_BYTE | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            64,
            64,
            0,
            null(),
        )
    });
    assert_ne!(
        server.0,
        INVALID_HANDLE_VALUE,
        "{}",
        error("CreateNamedPipeW")
    );
    // Open the client before the first server I/O, connecting the local pipe.
    let client = unsafe {
        CreateFileW(
            path.as_ptr(),
            FILE_GENERIC_READ | FILE_WRITE_DATA,
            0,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        )
    };
    assert_ne!(client, INVALID_HANDLE_VALUE, "{}", error("CreateFileW"));
    let client = unsafe { File::from_raw_handle(client.cast()) };
    (server, client)
}

#[test]
fn rearmed_read_and_cancelled_read_do_not_lose_the_next_frame() {
    let (server, mut client) = pipe();
    let deadline = Instant::now() + Duration::from_secs(2);
    let (cancelled, _) = Reply::request(&server, vec![0; 1], vec![1], deadline, || Ok(())).unwrap();
    assert_eq!(
        unsafe { WaitForSingleObject(cancelled.read.event.0, 0) },
        WAIT_TIMEOUT
    );
    let mut sent = [0];
    client.read_exact(&mut sent).unwrap();
    assert_eq!(sent, [1]);
    // No frame has arrived. Dropping the request must cancel the pending read.
    drop(cancelled);

    let (mut reply, written) =
        Reply::request(&server, vec![0; 1], vec![2], deadline, || Ok(())).unwrap();
    assert_eq!(written, 1);
    client.read_exact(&mut sent).unwrap();
    assert_eq!(sent, [2]);
    for frame in [42, 43] {
        client.write_all(&[frame]).unwrap();
        assert_eq!(reply.next_frame().unwrap(), &[frame]);
    }
}

#[test]
fn completed_frame_is_rejected_if_the_guard_changes_during_wait() {
    let (server, mut client) = pipe();
    let deadline = Instant::now() + Duration::from_secs(2);
    let check_count = Cell::new(None);
    let (mut reply, _) = Reply::request(&server, vec![0; 1], vec![1], deadline, || {
        if let Some(count) = check_count.get() {
            if count == 1 {
                return Err("stale frame".into());
            }
            check_count.set(Some(count + 1));
        }
        Ok(())
    })
    .unwrap();
    client.write_all(&[42]).unwrap();
    // The before-wait check passes. Only the after-completion check sees cancellation.
    check_count.set(Some(0));
    assert_eq!(reply.next_frame().unwrap_err(), "stale frame");
}
