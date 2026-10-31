use super::*;
use pretty_assertions::assert_eq;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

async fn server() -> (PublicReader, String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut bytes = [0; 2048];
                let length = socket.read(&mut bytes).await.unwrap();
                let request = String::from_utf8_lossy(&bytes[..length]);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let response = match path {
                    "/redirect" => "HTTP/1.1 302 Found\r\nLocation: /body\r\nContent-Length: 0\r\n\r\n".to_owned(),
                    "/private" => "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1/private\r\nContent-Length: 0\r\n\r\n".to_owned(),
                    "/loop" => "HTTP/1.1 302 Found\r\nLocation: /loop\r\nContent-Length: 0\r\n\r\n".to_owned(),
                    "/binary" => "HTTP/1.1 200 OK\r\nContent-Type: application/pdf\r\nContent-Length: 3\r\n\r\npdf".to_owned(),
                    "/unicode" => "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 8\r\n\r\néééé".to_owned(),
                    _ => "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 10\r\n\r\n0123456789".to_owned(),
                };
                socket.write_all(response.as_bytes()).await.unwrap();
            });
        }
    });
    let reader = PublicReader {
        test_addresses: [("fixture.example".into(), address)].into(),
    };
    (
        reader,
        format!("http://fixture.example:{}", address.port()),
        task,
    )
}

#[tokio::test]
async fn redirects_preserve_actual_final_url_and_enforce_byte_and_character_limits() {
    let (reader, url, task) = server().await;
    let limits = Limits {
        max_bytes: 5,
        ..Limits::default()
    };
    let page = reader
        .read(&format!("{url}/redirect"), &limits)
        .await
        .unwrap();
    assert_eq!(page.final_url, format!("{url}/body"));
    assert_eq!(page.content, "01234");
    assert!(page.truncated);
    let page = reader
        .read(
            &format!("{url}/unicode"),
            &Limits {
                max_chars: 3,
                ..Limits::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(page.content, "ééé");
    assert!(page.truncated);
    let exact = reader
        .read(
            &format!("{url}/body"),
            &Limits {
                max_bytes: 10,
                max_chars: 10,
                ..Limits::default()
            },
        )
        .await
        .unwrap();
    assert!(!exact.truncated);
    task.abort();
}

#[tokio::test]
async fn each_redirect_boundary_rejects_private_destinations_and_caps_hops() {
    let (reader, url, task) = server().await;
    assert_eq!(
        reader
            .read(&format!("{url}/private"), &Limits::default())
            .await
            .unwrap_err()
            .code,
        "non-public-url"
    );
    assert_eq!(
        reader
            .read(
                &format!("{url}/loop"),
                &Limits {
                    max_redirects: 1,
                    ..Limits::default()
                }
            )
            .await
            .unwrap_err()
            .code,
        "redirect-limit"
    );
    assert_eq!(
        reader
            .read(&format!("{url}/binary"), &Limits::default())
            .await
            .unwrap_err()
            .code,
        "unsupported-content"
    );
    task.abort();
}

#[test]
fn public_destination_policy_covers_special_use_and_literal_bypasses() {
    for ip in [
        "0.0.0.0",
        "10.1.2.3",
        "100.64.0.1",
        "127.0.0.1",
        "169.254.169.254",
        "172.16.0.1",
        "192.168.1.1",
        "192.0.2.1",
        "198.18.0.1",
        "198.51.100.1",
        "203.0.113.1",
        "224.0.0.1",
        "::1",
        "::ffff:127.0.0.1",
        "fc00::1",
        "fe80::1",
        "2001:db8::1",
        "2002:7f00:1::1",
    ] {
        assert!(!is_public_ip(ip.parse().unwrap()), "{ip}");
    }
    for ip in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111"] {
        assert!(is_public_ip(ip.parse().unwrap()));
    }
    for url in [
        "http://2130706433/",
        "http://0x7f000001/",
        "http://[::ffff:127.0.0.1]/",
        "file:///etc/passwd",
        "http://user:password@example.com/",
    ] {
        assert!(parse_public_url(url).is_err(), "{url}");
    }
}
