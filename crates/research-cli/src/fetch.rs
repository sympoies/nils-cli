//! Public HTTP(S) reader. Validation and DNS pinning repeat at every redirect.
use crate::{Limits, ResearchError};
use reqwest::{Url, redirect::Policy};
use std::{
    future::Future,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    time::Duration,
};

pub type ReadFuture<'a> = Pin<Box<dyn Future<Output = Result<Page, ResearchError>> + Send + 'a>>;
pub trait UrlReader: Send + Sync {
    fn read<'a>(&'a self, url: &'a str, limits: &'a Limits) -> ReadFuture<'a>;
}
#[derive(Debug, Clone)]
pub struct Page {
    pub url: String,
    pub final_url: String,
    pub content: String,
    pub truncated: bool,
}
#[derive(Default)]
pub struct PublicReader {
    #[cfg(test)]
    pub(crate) test_addresses: std::collections::HashMap<String, SocketAddr>,
}
impl UrlReader for PublicReader {
    fn read<'a>(&'a self, url: &'a str, limits: &'a Limits) -> ReadFuture<'a> {
        Box::pin(async move {
            tokio::time::timeout(
                Duration::from_secs(limits.timeout_seconds),
                self.fetch(url, limits),
            )
            .await
            .map_err(|_| ResearchError::new("timeout", "Public URL deadline exceeded"))?
        })
    }
}
impl PublicReader {
    async fn fetch(&self, original: &str, limits: &Limits) -> Result<Page, ResearchError> {
        let mut url = parse_public_url(original)?;
        for hop in 0..=limits.max_redirects {
            let host = url
                .host_str()
                .expect("validated host")
                .trim_matches(['[', ']']);
            let port = url.port_or_known_default().expect("http port");
            let addresses = self.addresses(host, port).await?;
            // New client prevents reuse of a pool entry resolved at a previous boundary.
            let client = reqwest::Client::builder()
                .no_proxy()
                .retry(reqwest::retry::never())
                .redirect(Policy::none())
                .timeout(Duration::from_secs(limits.timeout_seconds))
                .resolve_to_addrs(host, &addresses)
                .build()
                .map_err(|_| {
                    ResearchError::new("fetch-error", "Cannot create public HTTP client")
                })?;
            let mut response = client.get(url.clone()).send().await.map_err(fetch_error)?;
            if response.status().is_redirection() {
                if hop == limits.max_redirects {
                    return Err(ResearchError::new(
                        "redirect-limit",
                        "Public URL redirect limit exceeded",
                    ));
                }
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|s| s.to_str().ok())
                    .ok_or_else(|| {
                        ResearchError::new("fetch-error", "Redirect has no valid Location")
                    })?;
                let next = url
                    .join(location)
                    .map_err(|_| ResearchError::new("invalid-url", "Invalid redirect URL"))?;
                url = parse_public_url(next.as_str())?;
                continue;
            }
            if !response.status().is_success() {
                let mut error = ResearchError::new(
                    "fetch-error",
                    "Public URL returned an unsuccessful HTTP status",
                );
                error.details = Some(serde_json::json!({"http_status":response.status().as_u16()}));
                return Err(error);
            }
            let media = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !media.is_empty()
                && !media.starts_with("text/")
                && !media.starts_with("application/json")
                && !media.starts_with("application/xhtml+xml")
            {
                return Err(ResearchError::new(
                    "unsupported-content",
                    "Only text, HTML and JSON content can be read",
                ));
            }
            let mut bytes = Vec::new();
            let mut truncated = false;
            while let Some(chunk) = response.chunk().await.map_err(fetch_error)? {
                let remaining = limits.max_bytes - bytes.len();
                if chunk.len() > remaining {
                    bytes.extend_from_slice(&chunk[..remaining]);
                    truncated = true;
                    break;
                }
                bytes.extend_from_slice(&chunk);
            }
            let text = String::from_utf8_lossy(&bytes);
            truncated |= text.chars().count() > limits.max_chars;
            let content = text.chars().take(limits.max_chars).collect();
            return Ok(Page {
                url: original.into(),
                final_url: url.to_string(),
                content,
                truncated,
            });
        }
        unreachable!("bounded redirect loop returns at its final hop")
    }
    async fn addresses(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, ResearchError> {
        #[cfg(test)]
        if let Some(address) = self.test_addresses.get(host) {
            return Ok(vec![*address]);
        }
        let addresses: Vec<_> = tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| ResearchError::new("fetch-error", "Public URL DNS lookup failed"))?
            .collect();
        if addresses.is_empty() || addresses.iter().any(|a| !is_public_ip(a.ip())) {
            return Err(ResearchError::new(
                "non-public-url",
                "URL resolves to a non-public destination",
            ));
        }
        Ok(addresses)
    }
}
pub fn parse_public_url(raw: &str) -> Result<Url, ResearchError> {
    let url =
        Url::parse(raw).map_err(|_| ResearchError::new("invalid-url", "Invalid HTTP(S) URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(ResearchError::new(
            "invalid-url",
            "Only credential-free HTTP(S) URLs are allowed",
        ));
    }
    let host = url.host_str().expect("host").trim_matches(['[', ']']);
    if host.eq_ignore_ascii_case("localhost")
        || host.ends_with(".localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| !is_public_ip(ip))
    {
        return Err(ResearchError::new(
            "non-public-url",
            "Non-public URL destination is forbidden",
        ));
    }
    Ok(url)
}
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !matches!(a, 0 | 10 | 127 | 224..=255)
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 169 && b == 254)
                && !(a == 172 && (16..=31).contains(&b))
                && !(a == 192 && (b == 0 || b == 168 || (b == 88 && c == 99)))
                && !(a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
                && !(a == 203 && b == 0 && c == 113)
        }
        IpAddr::V6(ip) => {
            let s = ip.segments();
            // Only global unicast; exclude special-use assignments and documentation.
            (s[0] & 0xe000) == 0x2000
                && !(s[0] == 0x2001 && s[1] < 0x200)
                && !(s[0] == 0x2001 && s[1] == 0xdb8)
                && s[0] != 0x2002
                && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    }
}
fn fetch_error(error: reqwest::Error) -> ResearchError {
    if error.is_timeout() {
        ResearchError::new("timeout", "Public URL deadline exceeded")
    } else {
        ResearchError::new("fetch-error", "Public URL transport failed")
    }
}

#[cfg(test)]
#[path = "fetch_tests.rs"]
mod tests;
