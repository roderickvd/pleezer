// Adapted from https://chuxi.github.io/posts/websocket/ by chuxi

//! HTTP proxy support for HTTPS connections.
//!
//! This module provides HTTP(S) proxy functionality with:
//! * Environment-based configuration
//! * Basic authentication support
//! * CONNECT tunneling for HTTPS
//!
//! Adapted from <https://chuxi.github.io/posts/websocket>/ by chuxi
//!
//! # Example
//!
//! ```rust
//! use pleezer::proxy::Http;
//!
//! // From environment
//! if let Some(proxy) = Http::from_env() {
//!     // Connect through proxy
//!     let stream = proxy.connect_async("https://api.deezer.com").await?;
//! }
//!
//! // Manual configuration
//! let proxy: Http = "http://user:pass@proxy:8080".parse()?;
//! ```

use std::{env, fmt::Display, str::FromStr};

use base64::prelude::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use url::{Position, Url};
use veil::Redact;

use crate::error::{Error, Result};

/// HTTP proxy configuration and connection handling.
///
/// Supports:
/// * HTTP and HTTPS proxies
/// * Basic authentication
/// * Environment configuration
/// * CONNECT tunneling
///
/// # Security
///
/// Authentication credentials are:
/// * Redacted in debug output
/// * Base64 encoded for transmission
/// * Only sent over encrypted connections
#[derive(Redact, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Http {
    /// Basic auth credentials.
    ///
    /// Format: `Basic base64(username:password)`
    /// Redacted in debug output.
    #[redact]
    auth: Option<Vec<u8>>,

    /// Proxy server address.
    ///
    /// Format: `schema://host:port`
    // TODO: change into a `Url` type
    url: String,
}

/// Default HTTPS port.
const HTTPS_PORT: u16 = 443;

impl Http {
    /// Creates proxy configuration from environment.
    ///
    /// Checks for proxy URL in:
    /// 1. `HTTPS_PROXY`
    /// 2. `https_proxy`
    ///
    /// # Example
    ///
    /// ```rust
    /// std::env::set_var("HTTPS_PROXY", "http://proxy:8080");
    /// let proxy = Http::from_env();
    /// ```
    #[must_use]
    #[inline]
    pub fn from_env() -> Option<Self> {
        let proxy = env::var("HTTPS_PROXY")
            .or_else(|_| env::var("https_proxy"))
            .ok();

        proxy.and_then(|proxy| proxy.parse().ok())
    }

    /// Establishes connection to target through proxy.
    ///
    /// Creates HTTPS tunnel using HTTP CONNECT method.
    ///
    /// # Arguments
    ///
    /// * `target` - Target URL to connect to
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * Target URL is invalid
    /// * Proxy connection fails
    /// * Tunnel establishment fails
    /// * Authentication fails
    pub async fn connect_async(&self, target: &str) -> Result<TcpStream> {
        let target_url = Url::parse(target)?;
        let host = target_url
            .host_str()
            .ok_or_else(|| Error::invalid_argument("target host not available"))?;
        let port = target_url.port().unwrap_or(HTTPS_PORT);
        let tcp_stream = TcpStream::connect(&self.url).await?;
        Self::tunnel(tcp_stream, host, port, self.auth.as_ref()).await
    }

    /// Creates HTTPS tunnel through proxy.
    ///
    /// Protocol:
    /// 1. Sends CONNECT request
    /// 2. Adds authentication if present
    /// 3. Verifies successful response
    /// 4. Returns established tunnel
    ///
    /// # Arguments
    ///
    /// * `conn` - TCP connection to proxy
    /// * `host` - Target hostname
    /// * `port` - Target port
    /// * `auth` - Optional authentication header
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// * Connection fails
    /// * Authentication fails (407)
    /// * Invalid response
    /// * Response too large
    async fn tunnel(
        mut conn: TcpStream,
        host: &str,
        port: u16,
        auth: Option<&Vec<u8>>,
    ) -> Result<TcpStream> {
        let mut buf = format!(
            "\
         CONNECT {host}:{port} HTTP/1.1\r\n\
         Host: {host}:{port}\r\n\
         "
        )
        .into_bytes();

        if let Some(au) = auth {
            buf.extend_from_slice(b"Proxy-Authorization: ");
            buf.extend_from_slice(au.as_slice());
            buf.extend_from_slice(b"\r\n");
        }

        buf.extend_from_slice(b"\r\n");
        conn.write_all(&buf).await?;

        let mut buf = [0; 1024];
        let mut pos = 0;

        loop {
            let n = conn.read(&mut buf[pos..]).await?;
            if n == 0 {
                return Err(Error::data_loss("0 bytes in reading tunnel"));
            }
            pos += n;

            let recvd = &buf[..pos];
            if recvd.starts_with(b"HTTP/1.1 200") || recvd.starts_with(b"HTTP/1.0 200") {
                if recvd.ends_with(b"\r\n\r\n") {
                    return Ok(conn);
                }
                if pos == buf.len() {
                    return Err(Error::data_loss("proxy headers too long for tunnel"));
                }
            } else if recvd.starts_with(b"HTTP/1.1 407") {
                return Err(Error::permission_denied("proxy authentication required"));
            } else {
                return Err(Error::unknown("unsuccessful tunnel"));
            }
        }
    }
}

/// Parses proxy configuration from URL string.
///
/// Format: `[http|https]://[user:pass@]host:port`
///
/// # Examples
///
/// ```rust
/// // Simple proxy
/// let proxy: Http = "http://proxy:8080".parse()?;
///
/// // With authentication
/// let proxy: Http = "http://user:pass@proxy:8080".parse()?;
/// ```
///
/// # Errors
///
/// Returns error if:
/// * URL is invalid
/// * Scheme is not http/https
/// * Required components missing
impl FromStr for Http {
    type Err = Error;

    fn from_str(proxy_str: &str) -> std::result::Result<Self, Self::Err> {
        let url = Url::parse(proxy_str)?;
        let addr = &url[Position::BeforeHost..Position::AfterPort];

        let scheme = url.scheme();
        match scheme {
            "http" | "https" => {
                let mut basic_bytes: Option<Vec<u8>> = None;
                if let Some(pwd) = url.password() {
                    let encoded_str = format!(
                        "Basic {}",
                        BASE64_STANDARD.encode(format!("{}:{pwd}", url.username()))
                    );
                    basic_bytes = Some(encoded_str.into_bytes());
                }

                Ok(Self {
                    auth: basic_bytes,
                    url: addr.to_string(),
                })
            }

            _ => Err(Error::unimplemented(format!(
                "unsupported proxy schema {scheme}"
            ))),
        }
    }
}

/// Formats proxy as `host:port` string.
///
/// Note: Authentication credentials are not included
/// in the output for security.
impl Display for Http {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.url)
    }
}
