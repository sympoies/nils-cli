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
    pub body: String,
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
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    let mut buffer = Vec::new();
    let mut temp = [0u8; 8192];

    loop {
        let n = stream.read(&mut temp)?;
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

    let (method, path, body) = parse_request(&buffer, stream)?;
    requests
        .lock()
        .expect("requests lock")
        .push(RecordedRequest {
            method: method.clone(),
            path: path.clone(),
            body: body.clone(),
        });

    let route_key = RouteKey {
        method: method.to_ascii_uppercase(),
        path: path.clone(),
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

fn parse_request(buffer: &[u8], stream: &mut TcpStream) -> io::Result<(String, String, String)> {
    let mut headers_end = None;
    for i in 0..buffer.len().saturating_sub(3) {
        if &buffer[i..i + 4] == b"\r\n\r\n" {
            headers_end = Some(i);
            break;
        }
    }

    let header_end = headers_end.unwrap_or(buffer.len());
    let headers_raw = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = headers_raw.lines();
    let first = lines.next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let path = parts.next().unwrap_or("/").to_string();

    let mut content_length: usize = 0;
    for line in lines {
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            content_length = rest.trim().parse::<usize>().unwrap_or(0);
        }
    }

    let mut body = Vec::new();
    let body_start = header_end.saturating_add(4);
    if buffer.len() > body_start {
        body.extend_from_slice(&buffer[body_start..]);
    }

    while body.len() < content_length {
        let mut tmp = vec![0u8; content_length - body.len()];
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&tmp[..n]),
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
            Err(err) if err.kind() == io::ErrorKind::TimedOut => break,
            Err(err) => return Err(err),
        }
    }

    let body_str = String::from_utf8_lossy(&body).to_string();
    Ok((method, path, body_str))
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
