use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use nils_test_support::http::{HttpResponse, LoopbackServer, TestServer};
use pretty_assertions::assert_eq;

fn connect_url(url: &str) -> TcpStream {
    let addr = url.strip_prefix("http://").unwrap_or(url);
    TcpStream::connect(addr).expect("connect")
}

fn read_response(stream: &mut TcpStream) -> Vec<u8> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let mut content_len: Option<usize> = None;
    let mut header_end: Option<usize> = None;

    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if header_end.is_none()
                    && let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n")
                {
                    header_end = Some(pos + 4);
                    let header_text = String::from_utf8_lossy(&buf[..pos]);
                    for line in header_text.split("\r\n") {
                        if let Some((k, v)) = line.split_once(':')
                            && k.trim().eq_ignore_ascii_case("content-length")
                        {
                            content_len = v.trim().parse::<usize>().ok();
                        }
                    }
                }
                if let (Some(end), Some(len)) = (header_end, content_len)
                    && buf.len() >= end + len
                {
                    break;
                }
            }
            Err(err)
                if err.kind() == std::io::ErrorKind::TimedOut
                    || err.kind() == std::io::ErrorKind::WouldBlock =>
            {
                break;
            }
            Err(_) => break,
        }
    }

    buf
}

#[test]
fn loopback_server_captures_headers_and_body() {
    let server = LoopbackServer::new().expect("server");
    server.add_route("POST", "/submit", HttpResponse::new(200, "ok"));

    let body = "hello world";
    let request = format!(
        "POST /submit HTTP/1.1\r\nHost: localhost\r\nX-Test: Value\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );

    let mut stream = connect_url(&server.url());
    stream.write_all(request.as_bytes()).expect("write request");
    let _ = read_response(&mut stream);

    let requests = server.take_requests();
    assert_eq!(requests.len(), 1);
    let req = &requests[0];
    assert_eq!(req.method, "POST");
    assert_eq!(req.path, "/submit");
    assert_eq!(req.body, body.as_bytes());
    assert!(
        req.headers
            .iter()
            .any(|(k, v)| k == "x-test" && v == "Value")
    );
}

#[test]
fn test_server_uses_handler_and_records_request() {
    let server = TestServer::new(|req| {
        if req.path == "/ok" {
            HttpResponse::new(201, "created").with_header("X-Reply", "yes")
        } else {
            HttpResponse::new(404, "nope")
        }
    })
    .expect("server");

    let mut stream = connect_url(&server.url());
    let request = "GET /ok HTTP/1.1\r\nHost: localhost\r\nX-Client: tester\r\n\r\n";
    stream.write_all(request.as_bytes()).expect("write request");
    let response = read_response(&mut stream);
    let response_text = String::from_utf8_lossy(&response);
    assert!(response_text.starts_with("HTTP/1.1 201"));
    assert!(response_text.contains("X-Reply: yes"));

    let requests = server.take_requests();
    assert_eq!(requests.len(), 1);
    let req = &requests[0];
    assert_eq!(req.method, "GET");
    assert_eq!(req.path, "/ok");
    assert!(
        req.headers
            .iter()
            .any(|(k, v)| k == "x-client" && v == "tester")
    );
}
