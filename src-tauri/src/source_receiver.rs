//! Authenticated Chrome transport. All source decisions and writes stay in core.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Serialize;
use traeva_core::{AppError, ErrorCode, OrganizeService, SourceDownloadEvent, SourceState};

const MAX_HEADERS: usize = 8 * 1024;
const MAX_BODY: usize = 64 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Serialize)]
pub struct SourceConnection {
    pub enabled: bool,
    pub active: bool,
    pub endpoint: Option<String>,
    pub pairing_token: Option<String>,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct SourceReceiver {
    state: Mutex<ReceiverState>,
}

#[derive(Default)]
struct ReceiverState {
    running: Option<RunningReceiver>,
    error: Option<String>,
}

struct RunningReceiver {
    address: SocketAddr,
    token: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl SourceReceiver {
    pub fn restore(service: &Arc<OrganizeService>) -> Self {
        let receiver = Self::default();
        match service.source_capture_state() {
            Ok(state) if state.enabled => {
                let _ = receiver.set_enabled(service, true);
            }
            Ok(_) => {}
            Err(_) => receiver.remember_error("Source capture settings could not be read."),
        }
        receiver
    }

    pub fn set_enabled(
        &self,
        service: &Arc<OrganizeService>,
        enabled: bool,
    ) -> Result<SourceState, AppError> {
        let mut state = self.state.lock().map_err(|_| receiver_error())?;
        let source_state = service.set_source_enabled(enabled)?;
        if !enabled {
            state.running.take();
            state.error = None;
            return Ok(source_state);
        }
        if state
            .running
            .as_ref()
            .is_some_and(RunningReceiver::is_active)
        {
            return Ok(source_state);
        }
        state.running.take();
        let result = start_receiver(Arc::clone(service));
        match result {
            Ok(running) => {
                state.running = Some(running);
                state.error = None;
                Ok(source_state)
            }
            Err(error) => {
                state.error = Some(error.message.clone());
                let _ = service.set_source_enabled(false);
                Err(error)
            }
        }
    }

    pub fn connection(&self, enabled: bool) -> Result<SourceConnection, AppError> {
        let state = self.state.lock().map_err(|_| receiver_error())?;
        let running = state.running.as_ref().filter(|running| running.is_active());
        Ok(SourceConnection {
            enabled,
            active: running.is_some(),
            endpoint: running.map(|running| format!("http://{}", running.address)),
            pairing_token: running.map(|running| running.token.clone()),
            error: if state.running.is_some() && running.is_none() {
                Some("Chrome source capture stopped. Enable capture again to retry.".to_string())
            } else {
                state.error.clone()
            },
        })
    }

    fn remember_error(&self, message: &str) {
        if let Ok(mut state) = self.state.lock() {
            state.error = Some(message.to_string());
        }
    }
}

impl RunningReceiver {
    fn is_active(&self) -> bool {
        self.thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
    }
}

impl Drop for RunningReceiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Wake an idle nonblocking listener; the flag prevents dispatch.
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(50));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn start_receiver(service: Arc<OrganizeService>) -> Result<RunningReceiver, AppError> {
    let token = service.source_pairing_token()?;
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(receiver_error());
    }
    let port = service.source_receiver_port()?.unwrap_or(0);
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).map_err(|_| {
        AppError::observation(
            ErrorCode::FileUnreadable,
            "Chrome source capture could not open its saved local port. Close the conflicting app and enable capture again.",
        )
    })?;
    let address = listener.local_addr().map_err(|_| receiver_error())?;
    listener
        .set_nonblocking(true)
        .map_err(|_| receiver_error())?;
    service.set_source_receiver_port(address.port())?;
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let worker_token = token.clone();
    let thread = std::thread::Builder::new()
        .name("source-receiver".to_string())
        .spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, peer)) => {
                        if !peer.ip().is_loopback() || worker_stop.load(Ordering::Acquire) {
                            continue;
                        }
                        // Accepted sockets inherit O_NONBLOCK on macOS/BSD.
                        if stream.set_nonblocking(false).is_err() {
                            continue;
                        }
                        serve_connection(
                            &mut stream,
                            &worker_token,
                            address.port(),
                            &worker_stop,
                            &service,
                        );
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    Err(_) => break,
                }
            }
        })
        .map_err(|_| receiver_error())?;
    Ok(RunningReceiver {
        address,
        token,
        stop,
        thread: Some(thread),
    })
}

fn serve_connection(
    stream: &mut TcpStream,
    token: &str,
    port: u16,
    stop: &AtomicBool,
    service: &OrganizeService,
) {
    let _ = stream.set_write_timeout(Some(REQUEST_TIMEOUT));
    let request = match read_request(stream, token, port) {
        Ok(request) => request,
        Err(error) => {
            write_error(stream, error.status(), error.message());
            return;
        }
    };
    if stop.load(Ordering::Acquire) {
        write_error(stream, 503, "Source capture is stopping.");
        return;
    }
    match request {
        Request::State => match service.source_capture_state() {
            Ok(state) => write_json(stream, 200, &state),
            Err(_) => write_error(stream, 503, "Source capture settings could not be read."),
        },
        Request::Event(body) => {
            let event: SourceDownloadEvent = match serde_json::from_slice(&body) {
                Ok(event) => event,
                Err(_) => {
                    write_error(stream, 400, "Invalid source event.");
                    return;
                }
            };
            match service.ingest_source_event(event) {
                Ok(outcome) => write_json(stream, 200, &outcome),
                Err(error) if error.code == ErrorCode::InvalidInput => {
                    write_error(stream, 400, "Invalid source event.");
                }
                Err(_) => write_error(
                    stream,
                    503,
                    "Source saving could not be confirmed. Retry later.",
                ),
            }
        }
    }
}

enum Request {
    State,
    Event(Vec<u8>),
}

#[derive(Debug, PartialEq, Eq)]
enum RequestError {
    Invalid,
    Unauthorized,
    NotFound,
    TooLarge,
    Timeout,
}

impl RequestError {
    fn status(&self) -> u16 {
        match self {
            Self::Invalid => 400,
            Self::Unauthorized => 401,
            Self::NotFound => 404,
            Self::TooLarge => 413,
            Self::Timeout => 408,
        }
    }

    fn message(&self) -> &'static str {
        match self {
            Self::Invalid => "Invalid request.",
            Self::Unauthorized => "Pair this extension with Traeva before sending requests.",
            Self::NotFound => "Unknown source endpoint.",
            Self::TooLarge => "Source request is too large.",
            Self::Timeout => "Source request timed out.",
        }
    }
}

struct RequestHead {
    is_event: bool,
    length: usize,
}

fn parse_head(head: &[u8], token: &str, port: u16) -> Result<RequestHead, RequestError> {
    let text = std::str::from_utf8(head).map_err(|_| RequestError::Invalid)?;
    let mut lines = text.split("\r\n");
    let first = lines.next().ok_or(RequestError::Invalid)?;
    let is_event = match first {
        "GET /v1/source/state HTTP/1.1" => false,
        "POST /v1/source/events HTTP/1.1" => true,
        _ => return Err(RequestError::NotFound),
    };
    let mut host = None;
    let mut authorization = None;
    let mut content_length = None;
    let mut content_type = None;
    let mut origin = None;
    for line in lines {
        let (name, value) = line.split_once(':').ok_or(RequestError::Invalid)?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || value.bytes().any(|byte| byte < 32 || byte == 127)
        {
            return Err(RequestError::Invalid);
        }
        let value = value.trim();
        let slot = match name.to_ascii_lowercase().as_str() {
            "host" => &mut host,
            "authorization" => &mut authorization,
            "content-length" => &mut content_length,
            "content-type" => &mut content_type,
            "origin" => &mut origin,
            "transfer-encoding" | "expect" => return Err(RequestError::Invalid),
            _ => continue,
        };
        if slot.replace(value).is_some() {
            return Err(RequestError::Invalid);
        }
    }
    if host != Some(format!("127.0.0.1:{port}").as_str()) {
        return Err(RequestError::Invalid);
    }
    if let Some(origin) = origin {
        let id = origin
            .strip_prefix("chrome-extension://")
            .ok_or(RequestError::Unauthorized)?;
        if id.len() != 32 || !id.bytes().all(|byte| (b'a'..=b'p').contains(&byte)) {
            return Err(RequestError::Unauthorized);
        }
    }
    let supplied = authorization
        .and_then(|header| header.strip_prefix("Bearer "))
        .ok_or(RequestError::Unauthorized)?;
    if !token_matches(token.as_bytes(), supplied.as_bytes()) {
        return Err(RequestError::Unauthorized);
    }
    let length = match content_length {
        Some(value) if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) => {
            value.parse::<usize>().map_err(|_| RequestError::TooLarge)?
        }
        Some(_) => return Err(RequestError::Invalid),
        None if is_event => return Err(RequestError::Invalid),
        None => 0,
    };
    if length > MAX_BODY {
        return Err(RequestError::TooLarge);
    }
    if is_event {
        if length == 0
            || !content_type.is_some_and(|value| {
                value.eq_ignore_ascii_case("application/json")
                    || value.eq_ignore_ascii_case("application/json; charset=utf-8")
            })
        {
            return Err(RequestError::Invalid);
        }
    } else if length != 0 {
        return Err(RequestError::Invalid);
    }
    Ok(RequestHead { is_event, length })
}

fn token_matches(expected: &[u8], supplied: &[u8]) -> bool {
    if expected.len() != supplied.len() {
        return false;
    }
    expected
        .iter()
        .zip(supplied)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn read_request(stream: &mut TcpStream, token: &str, port: u16) -> Result<Request, RequestError> {
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    let mut bytes = Vec::new();
    let head_end = loop {
        if let Some(end) = bytes.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            if end > MAX_HEADERS {
                return Err(RequestError::TooLarge);
            }
            break end;
        }
        if bytes.len() > MAX_HEADERS {
            return Err(RequestError::TooLarge);
        }
        read_more(stream, &mut bytes, deadline)?;
    };
    let head = parse_head(&bytes[..head_end], token, port)?;
    let body_start = head_end + 4;
    while bytes.len() < body_start + head.length {
        read_more(stream, &mut bytes, deadline)?;
    }
    if bytes.len() != body_start + head.length {
        return Err(RequestError::Invalid);
    }
    if head.is_event {
        Ok(Request::Event(bytes.split_off(body_start)))
    } else {
        Ok(Request::State)
    }
}

fn read_more(
    stream: &mut TcpStream,
    bytes: &mut Vec<u8>,
    deadline: Instant,
) -> Result<(), RequestError> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or(RequestError::Timeout)?;
    stream
        .set_read_timeout(Some(remaining))
        .map_err(|_| RequestError::Invalid)?;
    let mut buffer = [0_u8; 4096];
    let read = stream
        .read(&mut buffer)
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => RequestError::Timeout,
            _ => RequestError::Invalid,
        })?;
    if read == 0 {
        return Err(RequestError::Invalid);
    }
    bytes.extend_from_slice(&buffer[..read]);
    Ok(())
}

fn write_json(stream: &mut TcpStream, status: u16, value: &impl Serialize) {
    match serde_json::to_vec(value) {
        Ok(body) => write_response(stream, status, &body),
        Err(_) => write_error(stream, 503, "Source response could not be prepared."),
    }
}

fn write_error(stream: &mut TcpStream, status: u16, message: &str) {
    write_json(stream, status, &serde_json::json!({ "error": message }));
}

fn write_response(stream: &mut TcpStream, status: u16, body: &[u8]) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        408 => "Request Timeout",
        413 => "Content Too Large",
        _ => "Service Unavailable",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\r\n",
        body.len()
    );
    if stream.write_all(head.as_bytes()).is_ok() {
        let _ = stream.write_all(body);
    }
}

fn receiver_error() -> AppError {
    AppError::observation(
        ErrorCode::FileUnreadable,
        "Chrome source capture could not start. Enable capture again to retry.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn head(extra: &str) -> String {
        format!(
            "POST /v1/source/events HTTP/1.1\r\nHost: 127.0.0.1:19827\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\nContent-Length: 2{extra}"
        )
    }

    #[test]
    fn requires_exact_auth_and_loopback_host() {
        let valid = head("");
        assert!(parse_head(valid.as_bytes(), TOKEN, 19827).is_ok());
        for invalid in [
            valid.replace(TOKEN, "wrong"),
            valid.replace("Bearer ", ""),
            head("\r\nOrigin: https://untrusted.test"),
            head("\r\nOrigin: null"),
        ] {
            assert_eq!(
                parse_head(invalid.as_bytes(), TOKEN, 19827).err(),
                Some(RequestError::Unauthorized)
            );
        }
        assert_eq!(
            parse_head(
                valid.replace("127.0.0.1", "untrusted.test").as_bytes(),
                TOKEN,
                19827
            )
            .err(),
            Some(RequestError::Invalid)
        );
        assert!(
            parse_head(
                head("\r\nOrigin: chrome-extension://abcdefghijklmnopabcdefghijklmnop").as_bytes(),
                TOKEN,
                19827
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_smuggling_oversize_and_non_json_bodies() {
        for extra in [
            "\r\nContent-Length: 2",
            "\r\nTransfer-Encoding: chunked",
            "\r\nExpect: 100-continue",
            "\r\nHost: 127.0.0.1:19827",
            "\r\nAuthorization: Bearer fake",
            "\r\n folded-header: value",
        ] {
            assert_eq!(
                parse_head(head(extra).as_bytes(), TOKEN, 19827).err(),
                Some(RequestError::Invalid)
            );
        }
        assert_eq!(
            parse_head(
                head("").replace("Length: 2", "Length: 65537").as_bytes(),
                TOKEN,
                19827
            )
            .err(),
            Some(RequestError::TooLarge)
        );
        assert_eq!(
            parse_head(
                head("")
                    .replace("application/json", "text/plain")
                    .as_bytes(),
                TOKEN,
                19827
            )
            .err(),
            Some(RequestError::Invalid)
        );
        assert_eq!(
            parse_head(
                head("").replace("Length: 2", "Length: +2").as_bytes(),
                TOKEN,
                19827
            )
            .err(),
            Some(RequestError::Invalid)
        );
    }

    #[test]
    fn state_has_no_body_and_routes_are_exact() {
        let get = format!(
            "GET /v1/source/state HTTP/1.1\r\nHost: 127.0.0.1:19827\r\nAuthorization: Bearer {TOKEN}"
        );
        assert!(parse_head(get.as_bytes(), TOKEN, 19827).is_ok());
        assert_eq!(
            parse_head(
                format!("{get}\r\nContent-Length: 2").as_bytes(),
                TOKEN,
                19827
            )
            .err(),
            Some(RequestError::Invalid)
        );
        assert_eq!(
            parse_head(
                get.replace("/state", "/state?token=secret").as_bytes(),
                TOKEN,
                19827
            )
            .err(),
            Some(RequestError::NotFound)
        );
    }

    #[test]
    fn reads_exact_body_over_real_loopback_socket() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            match read_request(&mut socket, TOKEN, address.port()).unwrap() {
                Request::Event(body) => assert_eq!(body, b"{}"),
                Request::State => panic!("expected event"),
            }
        });
        let mut client = TcpStream::connect(address).unwrap();
        client
            .write_all(
                format!(
                    "{}\r\n\r\n{{}}",
                    head("").replace("19827", &address.port().to_string())
                )
                .as_bytes(),
            )
            .unwrap();
        worker.join().unwrap();
    }

    fn get(connection: &SourceConnection, token: &str) -> String {
        let address = connection
            .endpoint
            .as_ref()
            .unwrap()
            .strip_prefix("http://")
            .unwrap();
        let mut socket = TcpStream::connect(address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(socket, "GET /v1/source/state HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\n\r\n").unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn receiver_is_opt_in_and_retains_pairing_across_restart() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("app-data");
        let service = Arc::new(OrganizeService::start(data.clone()));
        let receiver = SourceReceiver::restore(&service);
        let off = receiver.connection(false).unwrap();
        assert!(!off.active);
        assert!(off.pairing_token.is_none());
        receiver.set_enabled(&service, true).unwrap();
        let paired = receiver.connection(true).unwrap();
        let token = paired.pairing_token.as_ref().unwrap();
        assert_eq!(token.len(), 64);
        assert!(get(&paired, "wrong").starts_with("HTTP/1.1 401"));
        let response = get(&paired, token);
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("\"enabled\":true"));
        assert!(!response.contains(token));
        assert!(
            !response
                .to_ascii_lowercase()
                .contains("access-control-allow-origin")
        );
        drop(receiver);
        drop(service);

        let reopened = Arc::new(OrganizeService::start(data));
        let restored = SourceReceiver::restore(&reopened);
        let again = restored.connection(true).unwrap();
        assert!(again.active, "{:?}", again.error);
        assert_eq!(again.endpoint, paired.endpoint);
        assert_eq!(again.pairing_token, paired.pairing_token);
        restored.set_enabled(&reopened, false).unwrap();
        assert!(!reopened.source_capture_state().unwrap().enabled);
        assert!(!restored.connection(false).unwrap().active);
        assert!(
            TcpStream::connect(again.endpoint.unwrap().strip_prefix("http://").unwrap()).is_err()
        );
    }

    #[test]
    fn active_receiver_accepts_delayed_headers_and_body_on_macos() {
        let temp = tempfile::tempdir().unwrap();
        let service = Arc::new(OrganizeService::start(temp.path().join("app-data")));
        let receiver = SourceReceiver::restore(&service);
        receiver.set_enabled(&service, true).unwrap();
        let connection = receiver.connection(true).unwrap();
        let address = connection.endpoint.unwrap().replace("http://", "");
        let token = connection.pairing_token.unwrap();
        let mut client = TcpStream::connect(&address).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        // Let the nonblocking listener accept before any request bytes arrive.
        std::thread::sleep(Duration::from_millis(100));
        write!(
            client,
            "POST /v1/source/events HTTP/1.1\r\nHost: {address}\r\n"
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(100));
        write!(client, "Authorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n").unwrap();
        std::thread::sleep(Duration::from_millis(100));
        client.write_all(b"{}").unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        // Invalid event JSON shape, but the full HTTP request reached dispatch.
        assert!(response.starts_with("HTTP/1.1 400"), "{response}");
        assert!(response.contains("Invalid source event."));
    }

    #[test]
    fn active_receiver_times_out_an_incomplete_request_and_keeps_serving() {
        let temp = tempfile::tempdir().unwrap();
        let service = Arc::new(OrganizeService::start(temp.path().join("app-data")));
        let receiver = SourceReceiver::restore(&service);
        receiver.set_enabled(&service, true).unwrap();
        let connection = receiver.connection(true).unwrap();
        let address = connection.endpoint.as_ref().unwrap().replace("http://", "");
        let mut client = TcpStream::connect(address).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        client
            .write_all(b"GET /v1/source/state HTTP/1.1\r\n")
            .unwrap();
        let start = Instant::now();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 408"));
        assert!(start.elapsed() >= Duration::from_millis(1500));
        assert!(start.elapsed() < Duration::from_secs(4));
        assert!(
            get(&connection, connection.pairing_token.as_ref().unwrap())
                .starts_with("HTTP/1.1 200")
        );
    }

    #[test]
    fn occupied_saved_port_disables_capture_and_reports_failure() {
        let temp = tempfile::tempdir().unwrap();
        let service = Arc::new(OrganizeService::start(temp.path().join("app-data")));
        let occupied = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        service
            .set_source_receiver_port(occupied.local_addr().unwrap().port())
            .unwrap();
        service.set_source_enabled(true).unwrap();
        let receiver = SourceReceiver::restore(&service);
        assert!(!service.source_capture_state().unwrap().enabled);
        let connection = receiver.connection(false).unwrap();
        assert!(!connection.active);
        assert!(connection.error.is_some());
        assert!(connection.pairing_token.is_none());
    }

    #[test]
    fn incomplete_requests_have_a_total_read_deadline() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let start = Instant::now();
            let error = read_request(&mut socket, TOKEN, address.port()).err();
            assert_eq!(error, Some(RequestError::Timeout));
            assert!(start.elapsed() < Duration::from_secs(4));
        });
        let mut client = TcpStream::connect(address).unwrap();
        client
            .write_all(b"GET /v1/source/state HTTP/1.1\r\n")
            .unwrap();
        worker.join().unwrap();
    }
}
