use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use pretty_assertions::assert_eq;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection};
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;
use tungstenite::handshake::server::{
    Callback, ErrorResponse, Request as ServerRequest, Response as ServerResponse,
};

use super::*;

struct FixtureHandshake {
    redirect: Option<String>,
}

impl Callback for FixtureHandshake {
    fn on_request(
        self,
        request: &ServerRequest,
        response: ServerResponse,
    ) -> Result<ServerResponse, ErrorResponse> {
        assert_eq!(
            request.headers().get("authorization").unwrap(),
            "Bearer fixture-token"
        );
        match self.redirect {
            Some(location) => Err(ServerResponse::builder()
                .status(302)
                .header("Location", location)
                .body(None)
                .unwrap()),
            None => Ok(response),
        }
    }
}

fn trusted_config() -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from(
            include_bytes!("fixtures/ca.der").to_vec(),
        ))
        .unwrap();
    tls_config(roots)
}

fn spawn_tls_server() -> (u16, thread::JoinHandle<Option<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from(
            include_bytes!("fixtures/server.der").to_vec(),
        )],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
            include_bytes!("fixtures/server-key.der").to_vec(),
        )),
    )
    .unwrap();
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let stream = StreamOwned::new(ServerConnection::new(Arc::new(config)).unwrap(), stream);
        let Ok(mut socket) = tungstenite::accept_hdr(stream, FixtureHandshake { redirect: None })
        else {
            return None;
        };
        let name = socket.get_ref().conn.server_name().map(str::to_owned);
        socket.send(Message::Text("secure echo".into())).unwrap();
        let _ = socket.read();
        name
    });
    (port, handle)
}

fn request(url: &str) -> Request {
    let mut request = url.into_client_request().unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer fixture-token".parse().unwrap());
    request
}

fn certificate_error(error: &Error) -> &rustls::CertificateError {
    if let Error::Io(error) = error
        && let Some(rustls::Error::InvalidCertificate(certificate)) = error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<rustls::Error>())
    {
        return certificate;
    }
    panic!("expected a certificate verification error, got: {error}");
}

#[test]
fn trusted_tls_fixture_succeeds_with_sni_and_headers() {
    let (port, server) = spawn_tls_server();
    let (mut socket, _) = connect_with_config(
        request(&format!("wss://localhost:{port}/echo")),
        trusted_config(),
    )
    .unwrap_or_else(|error| panic!("trusted TLS connection failed: {error}"));
    assert_eq!(socket.read().unwrap().into_text().unwrap(), "secure echo");
    socket.close(None).unwrap();
    drop(socket);
    assert_eq!(server.join().unwrap().as_deref(), Some("localhost"));
}

#[test]
fn bundled_roots_reject_untrusted_tls_fixture() {
    let (port, server) = spawn_tls_server();
    let error = match connect(request(&format!("wss://localhost:{port}/echo"))) {
        Err(error) => error,
        Ok(_) => panic!("bundled Mozilla roots accepted the test-only CA"),
    };
    assert_eq!(
        certificate_error(&error),
        &rustls::CertificateError::UnknownIssuer
    );
    assert_eq!(server.join().unwrap(), None);
}

#[test]
fn trusted_ca_still_rejects_hostname_mismatch() {
    let (port, server) = spawn_tls_server();
    let error = match connect_with_config(
        request(&format!("wss://127.0.0.1:{port}/echo")),
        trusted_config(),
    ) {
        Err(error) => error,
        Ok(_) => panic!("TLS accepted an IP absent from the certificate's SANs"),
    };
    assert!(
        matches!(
            certificate_error(&error),
            rustls::CertificateError::NotValidForName
                | rustls::CertificateError::NotValidForNameContext { .. }
        ),
        "{error}"
    );
    assert_eq!(server.join().unwrap(), None);
}

fn spawn_redirects(location: Option<String>, count: usize) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/redirect", listener.local_addr().unwrap());
    let destination = location.unwrap_or_else(|| url.clone());
    let handle = thread::spawn(move || {
        for _ in 0..count {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let result = tungstenite::accept_hdr(
                stream,
                FixtureHandshake {
                    redirect: Some(destination.clone()),
                },
            );
            assert!(matches!(
                result,
                Err(HandshakeError::Failure(Error::Http(_)))
            ));
        }
    });
    (url, handle)
}

#[test]
fn redirect_to_tls_preserves_headers_and_validates_destination_sni() {
    let (port, tls_server) = spawn_tls_server();
    let (url, redirect_server) = spawn_redirects(Some(format!("wss://localhost:{port}/echo")), 1);
    let (mut socket, _) = connect_with_config(request(&url), trusted_config())
        .unwrap_or_else(|error| panic!("redirected TLS connection failed: {error}"));
    assert_eq!(socket.read().unwrap().into_text().unwrap(), "secure echo");
    socket.close(None).unwrap();
    drop(socket);
    redirect_server.join().unwrap();
    assert_eq!(tls_server.join().unwrap().as_deref(), Some("localhost"));
}

#[test]
fn redirect_limit_remains_three() {
    let (url, server) = spawn_redirects(None, 4);
    let error = match connect(request(&url)) {
        Err(error) => error,
        Ok(_) => panic!("redirect loop unexpectedly connected"),
    };
    assert!(matches!(error, Error::Http(response) if response.status() == 302));
    server.join().unwrap();
}

#[test]
fn tls_stream_retains_receive_timeout() {
    let (port, server) = spawn_tls_server();
    let (mut socket, _) = connect_with_config(
        request(&format!("wss://localhost:{port}/echo")),
        trusted_config(),
    )
    .unwrap_or_else(|error| panic!("trusted TLS connection failed: {error}"));
    assert_eq!(socket.read().unwrap().into_text().unwrap(), "secure echo");
    socket
        .get_ref()
        .tcp_stream()
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    assert!(
        matches!(socket.read(), Err(Error::Io(error)) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut))
    );
    drop(socket);
    assert_eq!(server.join().unwrap().as_deref(), Some("localhost"));
}

#[test]
fn runner_connect_timeout_bounds_a_stalled_handshake() {
    use crate::websocket::{runner::execute_websocket_request, schema::WebsocketRequestFile};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/stall", listener.local_addr().unwrap());
    let (release, wait) = std::sync::mpsc::channel();
    let server = thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        wait.recv_timeout(Duration::from_secs(5)).unwrap();
    });
    let directory = tempfile::TempDir::new().unwrap();
    let path = directory.path().join("stall.ws.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "url": url, "connectTimeoutSeconds": 1, "steps": [{"type": "close"}]
        }))
        .unwrap(),
    )
    .unwrap();
    let request = WebsocketRequestFile::load(&path).unwrap();
    let started = std::time::Instant::now();
    let error = execute_websocket_request(&request, "", None).expect_err("handshake must time out");
    release.send(()).unwrap();
    server.join().unwrap();
    assert!(
        format!("{error:#}").contains("connect timed out after 1s"),
        "{error:#}"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}
