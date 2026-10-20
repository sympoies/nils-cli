//! Blocking WebSocket connections with the current bundled Mozilla roots.
//!
//! Tungstenite's default TLS helper still selects webpki-roots 0.26. Keep TLS
//! configuration here so the handshake uses the supported 1.x trust anchors.
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;

use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use tungstenite::client::uri_mode;
use tungstenite::error::UrlError;
use tungstenite::handshake::client::{Request, Response};
use tungstenite::stream::Mode;
use tungstenite::{Error, HandshakeError, WebSocket};

pub(super) enum Transport {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl Transport {
    pub(super) fn tcp_stream(&self) -> &TcpStream {
        match self {
            Self::Plain(stream) => stream,
            Self::Tls(stream) => &stream.sock,
        }
    }
}

impl Read for Transport {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buf),
            Self::Tls(stream) => stream.read(buf),
        }
    }
}

impl Write for Transport {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buf),
            Self::Tls(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

fn tls_config(roots: RootCertStore) -> Arc<ClientConfig> {
    Arc::new(
        // Match reqwest's enabled provider without relying on process-global
        // installation order when workspace consumers also enable ring.
        ClientConfig::builder_with_provider(
            Arc::new(rustls::crypto::aws_lc_rs::default_provider()),
        )
        .with_safe_default_protocol_versions()
        .expect("default TLS versions are supported by the provider")
        .with_root_certificates(roots)
        .with_no_client_auth(),
    )
}

pub(super) fn connect(request: Request) -> tungstenite::Result<(WebSocket<Transport>, Response)> {
    let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    connect_with_config(request, tls_config(roots))
}

fn connect_with_config(
    mut request: Request,
    config: Arc<ClientConfig>,
) -> tungstenite::Result<(WebSocket<Transport>, Response)> {
    // Match tungstenite::connect: at most three redirects, preserving the
    // original request headers (including caller-supplied authorization).
    for attempt in 0..=3 {
        let mode = uri_mode(request.uri())?;
        let host = request.uri().host().ok_or(UrlError::NoHostName)?;
        let host = host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .unwrap_or(host);
        let port = request.uri().port_u16().unwrap_or(match mode {
            Mode::Plain => 80,
            Mode::Tls => 443,
        });
        let stream = (host, port)
            .to_socket_addrs()?
            .find_map(|addr| TcpStream::connect(addr).ok())
            .ok_or_else(|| UrlError::UnableToConnect(request.uri().to_string()))?;
        stream.set_nodelay(true)?;
        let stream = match mode {
            Mode::Plain => Transport::Plain(stream),
            Mode::Tls => {
                let name = rustls::pki_types::ServerName::try_from(host.to_owned())
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
                let client = ClientConnection::new(Arc::clone(&config), name)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                Transport::Tls(Box::new(StreamOwned::new(client, stream)))
            }
        };
        match tungstenite::client(request.clone(), stream) {
            Ok(pair) => return Ok(pair),
            Err(HandshakeError::Failure(Error::Http(response)))
                if response.status().is_redirection() && attempt < 3 =>
            {
                let Some(location) = response.headers().get("Location") else {
                    return Err(Error::Http(response));
                };
                *request.uri_mut() = location.to_str()?.parse()?;
            }
            Err(HandshakeError::Failure(err)) => return Err(err),
            Err(HandshakeError::Interrupted(_)) => {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "blocking websocket handshake interrupted",
                )
                .into());
            }
        }
    }
    unreachable!("redirect loop always returns on its last attempt")
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod tests;
