use crate::settings::wide;
use std::{
    ffi::c_void,
    fmt,
    fs::File,
    io::{self, Write},
    path::Path,
    ptr::{null, null_mut},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use windows_sys::{
    Win32::Networking::WinHttp::*,
    core::{BOOL, w},
};

#[derive(Debug)]
struct HttpStatus(u16);

impl fmt::Display for HttpStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "HTTP status {}", self.0)
    }
}

impl std::error::Error for HttpStatus {}

pub(super) fn status_code(error: &io::Error) -> Option<u16> {
    Some(error.get_ref()?.downcast_ref::<HttpStatus>()?.0)
}

pub(super) fn is_timeout(error: &io::Error) -> bool {
    error.raw_os_error() == Some(ERROR_WINHTTP_TIMEOUT as i32)
}

pub(super) fn is_connection_error(error: &io::Error) -> bool {
    status_code(error) == Some(407)
        || matches!(
            error.raw_os_error().map(|code| code as u32),
            Some(
                ERROR_WINHTTP_AUTODETECTION_FAILED
                    | ERROR_WINHTTP_AUTO_PROXY_SERVICE_ERROR
                    | ERROR_WINHTTP_BAD_AUTO_PROXY_SCRIPT
                    | ERROR_WINHTTP_UNABLE_TO_DOWNLOAD_SCRIPT
                    | ERROR_WINHTTP_CANNOT_CONNECT
                    | ERROR_WINHTTP_CONNECTION_ERROR
                    | ERROR_WINHTTP_LOGIN_FAILURE
                    | ERROR_WINHTTP_NAME_NOT_RESOLVED
            )
        )
}

struct Handle(*mut c_void);

impl Handle {
    fn new(raw: *mut c_void) -> io::Result<Self> {
        if raw.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(raw))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { WinHttpCloseHandle(self.0) };
    }
}

fn check(ok: BOOL) -> io::Result<()> {
    if ok != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

struct Response {
    request: Handle,
    _connection: Handle,
    _session: Handle,
}

impl Response {
    fn open(url: &str, timeout: i32, access_type: WINHTTP_ACCESS_TYPE) -> io::Result<Self> {
        let url = wide(url);
        let mut host = vec![0; url.len()];
        let mut resource = vec![0; url.len()];
        let mut extra = vec![0; url.len()];
        let mut parts = URL_COMPONENTS {
            dwStructSize: size_of::<URL_COMPONENTS>() as u32,
            lpszHostName: host.as_mut_ptr(),
            dwHostNameLength: host.len() as u32,
            lpszUrlPath: resource.as_mut_ptr(),
            dwUrlPathLength: resource.len() as u32,
            lpszExtraInfo: extra.as_mut_ptr(),
            dwExtraInfoLength: extra.len() as u32,
            ..Default::default()
        };
        unsafe {
            check(WinHttpCrackUrl(
                url.as_ptr(),
                0,
                ICU_REJECT_USERPWD,
                &mut parts,
            ))?;
            resource.truncate(parts.dwUrlPathLength as usize);
            resource.extend_from_slice(&extra[..parts.dwExtraInfoLength as usize]);
            resource.push(0);
            let flags = if parts.nScheme == WINHTTP_INTERNET_SCHEME_HTTPS {
                WINHTTP_FLAG_SECURE
            } else {
                0
            };
            let session = Handle::new(WinHttpOpen(
                w!("PowerDock-updater"),
                access_type,
                null(),
                null(),
                0,
            ))?;
            check(WinHttpSetTimeouts(
                session.0, timeout, timeout, timeout, timeout,
            ))?;
            let header_timeout = timeout as u32;
            check(WinHttpSetOption(
                session.0,
                WINHTTP_OPTION_RECEIVE_RESPONSE_TIMEOUT,
                (&header_timeout as *const u32).cast(),
                size_of::<u32>() as u32,
            ))?;
            let connection = Handle::new(WinHttpConnect(session.0, host.as_ptr(), parts.nPort, 0))?;
            let accept = [w!("application/vnd.github+json"), null()];
            let request = Handle::new(WinHttpOpenRequest(
                connection.0,
                w!("GET"),
                resource.as_ptr(),
                null(),
                null(),
                accept.as_ptr(),
                flags,
            ))?;
            check(WinHttpSendRequest(request.0, null(), 0, null(), 0, 0, 0))?;
            check(WinHttpReceiveResponse(request.0, null_mut()))?;
            let mut status = 0u32;
            let mut length = size_of::<u32>() as u32;
            check(WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                null(),
                (&mut status as *mut u32).cast(),
                &mut length,
                null_mut(),
            ))?;
            if !(200..300).contains(&status) {
                return Err(io::Error::other(HttpStatus(status as u16)));
            }
            Ok(Self {
                request,
                _connection: connection,
                _session: session,
            })
        }
    }

    fn read(&self, buffer: &mut [u8]) -> io::Result<usize> {
        let mut count = 0u32;
        unsafe {
            check(WinHttpReadData(
                self.request.0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut count,
            ))?;
        }
        Ok(count as usize)
    }
}

fn copy_response(
    url: &str,
    timeout: i32,
    access_type: WINHTTP_ACCESS_TYPE,
    mut output: impl Write,
    limit: u64,
    mut progress: impl FnMut(u64),
) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_millis(timeout as u64);
    let url = url.to_owned();
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::Builder::new().spawn(move || {
        let result = (|| -> io::Result<()> {
            let response = Response::open(&url, timeout, access_type)?;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let count = response.read(&mut buffer)?;
                if count == 0 || sender.send(Ok(buffer[..count].to_vec())).is_err() {
                    return Ok(());
                }
            }
        })();
        let _ = sender.send(result.map(|()| Vec::new()));
    })?;

    let mut written = 0;
    loop {
        let buffer = receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => io::ErrorKind::TimedOut.into(),
                mpsc::RecvTimeoutError::Disconnected => {
                    io::Error::other("Windows HTTP stopped before completing the response")
                }
            })??;
        if Instant::now() >= deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        if buffer.is_empty() {
            return Ok(());
        }
        written += buffer.len() as u64;
        if written > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Update download exceeded its size limit",
            ));
        }
        output.write_all(&buffer)?;
        progress(written);
    }
}

pub(super) fn get(url: &str, limit: usize) -> Result<String, velopack::Error> {
    let mut bytes = Vec::new();
    copy_response(
        url,
        10_000,
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
        &mut bytes,
        limit as u64,
        |_| {},
    )?;
    Ok(String::from_utf8(bytes).map_err(io::Error::other)?)
}

pub(super) fn download(
    url: &str,
    path: &Path,
    size: u64,
    mut progress: impl FnMut(i16),
) -> Result<(), velopack::Error> {
    let file = velopack::misc::retry_io(|| File::create(path))?;
    let mut last = 0;
    copy_response(
        url,
        240_000,
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
        file,
        size,
        |written| {
            let value = (written * 20 / size * 5) as i16;
            if value > last {
                last = value;
                progress(value);
            }
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        net::{TcpListener, TcpStream},
    };

    fn accept_request(listener: &TcpListener, target: &str) -> TcpStream {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(&mut socket);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.split_whitespace().nth(1), Some(target));
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                break;
            }
        }
        socket
    }

    fn respond(listener: &TcpListener, target: &str, response: &str) {
        accept_request(listener, target)
            .write_all(response.as_bytes())
            .unwrap();
    }

    #[test]
    fn windows_http_follows_redirects_and_preserves_http_status() {
        let origin = TcpListener::bind("127.0.0.1:0").unwrap();
        let cdn = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin_url = format!("http://{}", origin.local_addr().unwrap());
        let cdn_url = format!("http://{}", cdn.local_addr().unwrap());
        let server = thread::spawn(move || {
            respond(
                &origin,
                "/feed?preview=1",
                &format!(
                    "HTTP/1.1 302 Found\r\nLocation: {cdn_url}/asset?part=1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                ),
            );
            respond(
                &cdn,
                "/asset?part=1",
                "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nasset",
            );
            respond(
                &origin,
                "/latest",
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        });
        let mut asset = Vec::new();
        copy_response(
            &format!("{origin_url}/feed?preview=1"),
            3_000,
            WINHTTP_ACCESS_TYPE_NO_PROXY,
            &mut asset,
            5,
            |_| {},
        )
        .unwrap();
        assert_eq!(asset, b"asset");

        let error = Response::open(
            &format!("{origin_url}/latest"),
            3_000,
            WINHTTP_ACCESS_TYPE_NO_PROXY,
        )
        .err()
        .unwrap();
        assert_eq!(status_code(&error), Some(404));
        server.join().unwrap();
    }

    #[test]
    fn request_budget_includes_headers_and_late_eof() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/slow", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let mut socket = accept_request(&listener, "/slow");
            thread::sleep(Duration::from_millis(150));
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            thread::sleep(Duration::from_millis(425));
            let _ = socket.write_all(b"0\r\n\r\n");
        });
        let error = copy_response(
            &url,
            500,
            WINHTTP_ACCESS_TYPE_NO_PROXY,
            io::sink(),
            0,
            |_| {},
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        server.join().unwrap();
    }
}
