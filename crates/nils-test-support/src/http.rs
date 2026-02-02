use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RecordedRequest {
    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    pub fn header_value(&self, key: &str) -> Option<String> {
        let key = key.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k == &key)
            .map(|(_, v)| v.clone())
    }
}

#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
    pub headers: Vec<(String, String)>,
}

impl HttpResponse {
    pub fn new(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            body: body.into(),
            headers: Vec::new(),
        }
    }

    pub fn with_header(mut self, key: &str, value: &str) -> Self {
        self.headers.push((key.to_string(), value.to_string()));
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct RouteKey {
    method: String,
    path: String,
}

pub struct LoopbackServer {
    addr: SocketAddr,
    routes: Arc<Mutex<HashMap<RouteKey, HttpResponse>>>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl LoopbackServer {
    pub fn new() -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;

        let routes = Arc::new(Mutex::new(HashMap::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));

        let routes_t = Arc::clone(&routes);
        let requests_t = Arc::clone(&requests);
        let stop_t = Arc::clone(&stop);

        let handle = thread::spawn(move || {
            while !stop_t.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = handle_connection(&mut stream, &routes_t, &requests_t);
                    }
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                        thread::yield_now();
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            addr,
            routes,
            requests,
            stop,
            handle: Some(handle),
        })
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn add_route(&self, method: &str, path: &str, response: HttpResponse) {
        let key = RouteKey {
            method: method.trim().to_ascii_uppercase(),
            path: path.trim().to_string(),
        };
        let mut routes = self.routes.lock().expect("routes lock");
        routes.insert(key, response);
    }

    pub fn take_requests(&self) -> Vec<RecordedRequest> {
        let mut guard = self.requests.lock().expect("requests lock");
        std::mem::take(&mut *guard)
    }
}

impl Drop for LoopbackServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn handle_connection(
    stream: &mut TcpStream,
    routes: &Arc<Mutex<HashMap<RouteKey, HttpResponse>>>,
    requests: &Arc<Mutex<Vec<RecordedRequest>>>,
) -> io::Result<()> {
    let request = read_request(stream)?;
    requests
        .lock()
        .expect("requests lock")
        .push(request.clone());

    let route_key = RouteKey {
        method: request.method.to_ascii_uppercase(),
        path: request.path.clone(),
    };

    let response = routes
        .lock()
        .expect("routes lock")
        .get(&route_key)
        .cloned()
        .unwrap_or_else(|| HttpResponse::new(404, "not found"));

    write_response(stream, response)?;
    Ok(())
}

fn read_request(stream: &mut TcpStream) -> io::Result<RecordedRequest> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut buffer = Vec::new();
    let mut temp = [0u8; 8192];

    loop {
        let n = match stream.read(&mut temp) {
            Ok(n) => n,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => 0,
            Err(err) if err.kind() == io::ErrorKind::TimedOut => 0,
            Err(err) => return Err(err),
        };
        if n == 0 {
            break;
        }
        buffer.extend_from_slice(&temp[..n]);
        if buffer.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buffer.len() > 1024 * 1024 {
            break;
        }
    }

    let (method, path, headers, rest) = parse_headers_and_rest(&buffer);

    if header_value(&headers, "expect")
        .is_some_and(|v| v.to_ascii_lowercase().contains("100-continue"))
    {
        let _ = stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
        let _ = stream.flush();
    }

    let body = if let Some(te) =
        header_value(&headers, "transfer-encoding").map(|v| v.to_ascii_lowercase())
    {
        if te.contains("chunked") {
            read_chunked_body(stream, rest)
        } else {
            Vec::new()
        }
    } else if let Some(cl) = header_value(&headers, "content-length") {
        let len = cl.parse::<usize>().unwrap_or(0);
        read_exact_bytes(stream, rest, len)
    } else {
        rest
    };

    Ok(RecordedRequest {
        method,
        path,
        headers,
        body,
    })
}

fn parse_headers_and_rest(buffer: &[u8]) -> (String, String, Vec<(String, String)>, Vec<u8>) {
    let mut headers_end = None;
    for i in 0..buffer.len().saturating_sub(3) {
        if &buffer[i..i + 4] == b"\r\n\r\n" {
            headers_end = Some(i);
            break;
        }
    }

    let header_end = headers_end.unwrap_or(buffer.len());
    let headers_raw = String::from_utf8_lossy(&buffer[..header_end]);
    let mut lines = headers_raw.split("\r\n");
    let first = lines.next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let target = parts.next().unwrap_or("/");
    let path = target.split('?').next().unwrap_or("/").to_string();

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            headers.push((key, value));
        }
    }

    let body_start = header_end.saturating_add(4);
    let rest = if buffer.len() > body_start {
        buffer[body_start..].to_vec()
    } else {
        Vec::new()
    };

    (method, path, headers, rest)
}

fn header_value(headers: &[(String, String)], key: &str) -> Option<String> {
    let key = key.to_ascii_lowercase();
    headers
        .iter()
        .find(|(k, _)| k == &key)
        .map(|(_, v)| v.clone())
}

fn read_exact_bytes(stream: &mut TcpStream, mut already: Vec<u8>, want: usize) -> Vec<u8> {
    while already.len() < want {
        let mut tmp = vec![0u8; (want - already.len()).min(8192)];
        let n = stream.read(&mut tmp).unwrap_or_default();
        if n == 0 {
            break;
        }
        already.extend_from_slice(&tmp[..n]);
    }
    already.truncate(want);
    already
}

fn take_line(buf: &mut Vec<u8>, stream: &mut TcpStream) -> Option<Vec<u8>> {
    loop {
        if let Some(pos) = buf.windows(2).position(|w| w == b"\r\n") {
            let mut line = buf.drain(..pos).collect::<Vec<u8>>();
            let _ = buf.drain(..2);
            if line.ends_with(b"\r") {
                line.pop();
            }
            return Some(line);
        }

        let mut tmp = [0u8; 4096];
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
}

fn read_chunked_body(stream: &mut TcpStream, mut buf: Vec<u8>) -> Vec<u8> {
    let mut body = Vec::new();
    while let Some(line) = take_line(&mut buf, stream) {
        let line_str = String::from_utf8_lossy(&line);
        let size_str = line_str.split(';').next().unwrap_or("").trim();
        let Ok(size) = usize::from_str_radix(size_str, 16) else {
            break;
        };
        if size == 0 {
            while let Some(l) = take_line(&mut buf, stream) {
                if l.is_empty() {
                    break;
                }
            }
            break;
        }

        while buf.len() < size + 2 {
            let mut tmp = [0u8; 8192];
            let n = stream.read(&mut tmp).unwrap_or_default();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
        }

        if buf.len() < size + 2 {
            break;
        }
        body.extend_from_slice(&buf[..size]);
        buf.drain(..size + 2);
    }
    body
}

pub struct TestServer {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl TestServer {
    pub fn new<F>(handler: F) -> io::Result<Self>
    where
        F: Fn(&RecordedRequest) -> HttpResponse + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;

        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let handler = Arc::new(handler);

        let requests_t = Arc::clone(&requests);
        let stop_t = Arc::clone(&stop);
        let handler_t = Arc::clone(&handler);

        let handle = thread::spawn(move || {
            while !stop_t.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        if let Ok(request) = read_request(&mut stream) {
                            let response = handler_t(&request);
                            requests_t.lock().expect("requests lock").push(request);
                            let _ = write_response(&mut stream, response);
                        }
                    }
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                        thread::yield_now();
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            addr,
            requests,
            stop,
            handle: Some(handle),
        })
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn take_requests(&self) -> Vec<RecordedRequest> {
        let mut guard = self.requests.lock().expect("requests lock");
        std::mem::take(&mut *guard)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn write_response(stream: &mut TcpStream, response: HttpResponse) -> io::Result<()> {
    let status_text = match response.status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    };

    let body = response.body;
    let mut headers = response.headers;
    headers.push(("Content-Length".to_string(), body.len().to_string()));

    let mut out = String::new();
    out.push_str(&format!("HTTP/1.1 {} {}\r\n", response.status, status_text));
    for (k, v) in headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    out.push_str(&body);

    stream.write_all(out.as_bytes())?;
    stream.flush()?;
    Ok(())
}
