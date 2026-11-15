//! Talking to a samu server: HTTP/1.1 by hand, plain or over TLS
//! (ADR 0028 §2).
//!
//! One request per connection with `Connection: close` needs neither
//! keep-alive nor a client library (as in catastr-cli, `client.rs`). TLS
//! through rustls with the system's certificates, for
//! `https://samu.example.org`; on Android from the system's certificate
//! directories, as catastr-sync does.

pub mod profile;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use samu_api::{ApiError, ErrorCode};
use serde_json::Value;

/// What went wrong. The first three are what a caller acts on (the
/// command line's exit codes, the app's sign-in screen); the rest is said.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The server could not be reached.
    #[error("{0}")]
    NoServer(String),
    /// What was asked for does not exist.
    #[error("{0}")]
    NotFound(String),
    /// Not signed in, or the sign-in failed.
    #[error("{0}")]
    Unauthorized(String),
    /// The server refused, with its message.
    #[error("{0}")]
    Refused(String),
    /// An answer this client cannot read, or an address it cannot use.
    #[error("{0}")]
    Protocol(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("TLS: {0}")]
    Tls(#[from] rustls::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

fn protocol(message: impl Into<String>) -> Error {
    Error::Protocol(message.into())
}

/// Where the server is.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    tls: bool,
    host: String,
    port: u16,
}

impl Target {
    /// The `Host` header: the port too, unless it is the scheme's own
    /// (RFC 9110 §7.2). Without it a server that names its own address,
    /// as in a pairing code, names the wrong one.
    fn host_header(&self) -> String {
        let default = if self.tls { 443 } else { 80 };
        if self.port == default {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

fn parse(url: &str) -> Result<Target> {
    let (tls, rest) = if let Some(rest) = url.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        (false, rest)
    } else {
        return Err(protocol(format!(
            "the server {url:?} is neither http:// nor https://"
        )));
    };
    let authority = rest.trim_end_matches('/');
    if authority.contains('/') {
        return Err(protocol(format!(
            "the server {url:?} has a path; samu serves from the root"
        )));
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (
            host,
            port.parse()
                .map_err(|_| protocol(format!("the port of the server {url:?}")))?,
        ),
        None => (authority, if tls { 443 } else { 80 }),
    };
    if host.is_empty() {
        return Err(protocol(format!("the server {url:?} has no host")));
    }
    Ok(Target {
        tls,
        host: host.to_string(),
        port,
    })
}

/// An answer.
#[derive(Debug)]
pub struct Answer {
    pub status: u16,
    pub body: Vec<u8>,
    /// The answer's `Content-Type`, when it has one.
    pub content_type: Option<String>,
}

impl Answer {
    pub fn json(&self) -> Result<Value> {
        serde_json::from_slice(&self.body)
            .map_err(|_| protocol("the server answered something that is not JSON"))
    }
}

/// The certificates the system trusts.
#[cfg(not(target_os = "android"))]
fn system_certs() -> Vec<rustls::pki_types::CertificateDer<'static>> {
    rustls_native_certs::load_native_certs().certs
}

/// Where Android keeps the certificates it trusts, the updatable store
/// first (as catastr-sync, `peer.rs`): one PEM file per certificate.
#[cfg(target_os = "android")]
const ANDROID_CERT_DIRS: &[&str] = &[
    "/apex/com.android.conscrypt/cacerts",
    "/system/etc/security/cacerts",
];

#[cfg(target_os = "android")]
fn system_certs() -> Vec<rustls::pki_types::CertificateDer<'static>> {
    use rustls::pki_types::pem::PemObject;
    for dir in ANDROID_CERT_DIRS {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        let certs: Vec<_> = entries
            .flatten()
            .filter_map(|entry| std::fs::read(entry.path()).ok())
            .filter_map(|pem| rustls::pki_types::CertificateDer::from_pem_slice(&pem).ok())
            .collect();
        if !certs.is_empty() {
            return certs;
        }
    }
    Vec::new()
}

fn tls_config() -> Result<Arc<rustls::ClientConfig>> {
    let mut roots = rustls::RootCertStore::empty();
    for cert in system_certs() {
        let _ = roots.add(cert);
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Reads until the server closes. A TLS peer that closes without
/// `close_notify` ends the answer as well; the length is checked after.
fn read_all(stream: &mut impl Read) -> Result<Vec<u8>> {
    let mut raw = Vec::new();
    match stream.read_to_end(&mut raw) {
        Ok(_) => Ok(raw),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof && !raw.is_empty() => Ok(raw),
        Err(e) => Err(e.into()),
    }
}

fn dechunk(mut body: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let line_end = body
            .windows(2)
            .position(|w| w == b"\r\n")
            .ok_or_else(|| protocol("a broken chunked answer"))?;
        let size_text =
            std::str::from_utf8(&body[..line_end]).map_err(|_| protocol("a broken chunk size"))?;
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| protocol("a broken chunk size"))?;
        body = &body[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        let chunk = body
            .get(..size)
            .ok_or_else(|| protocol("a chunked answer cut short"))?;
        out.extend_from_slice(chunk);
        body = body.get(size + 2..).unwrap_or(&[]);
    }
}

fn split(raw: &[u8]) -> Result<Answer> {
    let head_end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| protocol("the server closed without an answer"))?;
    let head = std::str::from_utf8(&raw[..head_end])
        .map_err(|_| protocol("an answer head that is not text"))?;
    let status = head
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| protocol("an answer without a status"))?;
    let body = &raw[head_end + 4..];
    let chunked = head.lines().any(|line| {
        let line = line.to_ascii_lowercase();
        line.starts_with("transfer-encoding:") && line.contains("chunked")
    });
    let content_type = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-type")
            .then(|| value.trim().to_string())
    });
    Ok(Answer {
        status,
        content_type,
        body: if chunked {
            dechunk(body)?
        } else {
            body.to_vec()
        },
    })
}

/// The server at one address, with or without a session.
#[derive(Debug, Clone)]
pub struct Client {
    target: Target,
    token: Option<String>,
    /// For connecting and reading; long enough for `/events` by default.
    timeout: Duration,
}

impl Client {
    pub fn new(url: &str, token: Option<String>) -> Result<Self> {
        Ok(Self {
            target: parse(url)?,
            token,
            // The server sends a keep-alive on `/events` every 15 seconds.
            timeout: Duration::from_secs(60),
        })
    }

    /// A client that gives up sooner (shell completion, a phone's sync).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Opens a connection and writes one request on it. A failed connection
    /// is [`Error::NoServer`].
    fn open(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        accept: &str,
    ) -> Result<Box<dyn Stream>> {
        let body = body.map(Value::to_string).unwrap_or_default();
        self.open_raw(
            method,
            path,
            "application/json",
            &[],
            body.as_bytes(),
            accept,
        )
    }

    /// Like [`Client::open`], with a body of any type and extra headers.
    fn open_raw(
        &self,
        method: &str,
        path: &str,
        content_type: &str,
        headers: &[(&str, String)],
        body: &[u8],
        accept: &str,
    ) -> Result<Box<dyn Stream>> {
        let address = format!("{}:{}", self.target.host, self.target.port);
        let no_server = |e: std::io::Error| Error::NoServer(format!("cannot reach {address}: {e}"));
        let socket = std::net::ToSocketAddrs::to_socket_addrs(&address)
            .map_err(no_server)?
            .next()
            .ok_or_else(|| Error::NoServer(format!("cannot resolve {address}")))?;
        let tcp = TcpStream::connect_timeout(&socket, self.timeout).map_err(no_server)?;
        tcp.set_read_timeout(Some(self.timeout))?;
        let auth = self
            .token
            .as_ref()
            .map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
        let extra: String = headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}\r\n"))
            .collect();
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n{auth}{extra}\
             Accept: {accept}\r\nContent-Type: {content_type}\r\n\
             Content-Length: {}\r\nUser-Agent: samu/{}\r\n\r\n",
            self.target.host_header(),
            body.len(),
            env!("CARGO_PKG_VERSION"),
        );
        let mut stream: Box<dyn Stream> = if self.target.tls {
            let name = rustls::pki_types::ServerName::try_from(self.target.host.clone())
                .map_err(|_| protocol(format!("the host name {:?}", self.target.host)))?;
            let connection = rustls::ClientConnection::new(tls_config()?, name)?;
            Box::new(rustls::StreamOwned::new(connection, tcp))
        } else {
            Box::new(tcp)
        };
        stream.write_all(request.as_bytes()).map_err(no_server)?;
        stream.write_all(body).map_err(no_server)?;
        Ok(stream)
    }

    /// Sends a body of any type with extra headers, as a sender outside samu
    /// would (`samu hook send`), and turns a failure status into an error.
    pub fn call_raw(
        &self,
        path: &str,
        content_type: &str,
        headers: &[(&str, String)],
        body: &[u8],
    ) -> Result<Answer> {
        self.call_bytes("POST", path, content_type, headers, body)
    }

    /// Sends a body of any type, like a file to `PUT /blobs/{sha256}`
    /// (ADR 0039 §4), and turns a failure status into an error. The answer's
    /// body is bytes, so it also fetches a file.
    pub fn call_bytes(
        &self,
        method: &str,
        path: &str,
        content_type: &str,
        headers: &[(&str, String)],
        body: &[u8],
    ) -> Result<Answer> {
        let mut stream = self.open_raw(
            method,
            path,
            content_type,
            headers,
            body,
            "application/json",
        )?;
        let answer = split(&read_all(&mut stream)?)?;
        if (200..300).contains(&answer.status) {
            return Ok(answer);
        }
        Err(failure_of(answer.status, &answer.body))
    }

    /// Sends a body of any type and reads the whole answer, whatever its
    /// status, as a proxy needs it (ADR 0100 §9).
    pub fn exchange(
        &self,
        method: &str,
        path: &str,
        content_type: &str,
        body: &[u8],
    ) -> Result<Answer> {
        let mut stream = self.open_raw(method, path, content_type, &[], body, "*/*")?;
        split(&read_all(&mut stream)?)
    }

    /// Sends one request and reads the whole answer.
    pub fn send(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Answer> {
        let mut stream = self.open(method, path, body, "application/json")?;
        split(&read_all(&mut stream)?)
    }

    /// Opens `GET path` as a server-sent event stream and hands each event
    /// to `on_event` (name, data) until it returns `false` or the server
    /// closes.
    pub fn events<E: From<Error>>(
        &self,
        path: &str,
        mut on_event: impl FnMut(&str, &str) -> Result<bool, E>,
    ) -> Result<(), E> {
        let stream = self.open("GET", path, None, "text/event-stream")?;
        let mut reader = BufReader::new(stream);
        let mut status_line = String::new();
        reader.read_line(&mut status_line).map_err(Error::from)?;
        let status: u16 = status_line
            .split(' ')
            .nth(1)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| protocol("the server closed without an answer"))?;
        let mut chunked = false;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).map_err(Error::from)? == 0 {
                break;
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("transfer-encoding:") && lower.contains("chunked") {
                chunked = true;
            }
        }
        if status != 200 {
            let rest = read_all(&mut reader)?;
            let body = if chunked { dechunk(&rest)? } else { rest };
            return Err(failure_of(status, &body).into());
        }
        let mut parser = SseParser::default();
        let mut feed = |text: &str| -> Result<bool, E> {
            for line in text.split_inclusive('\n') {
                if let Some((event, data)) = parser.line(line)
                    && !on_event(&event, &data)?
                {
                    return Ok(false);
                }
            }
            Ok(true)
        };
        if chunked {
            loop {
                let mut size_line = String::new();
                if reader.read_line(&mut size_line).map_err(Error::from)? == 0 {
                    return Ok(());
                }
                let size =
                    usize::from_str_radix(size_line.trim().split(';').next().unwrap_or(""), 16)
                        .map_err(|_| protocol("a broken chunk size"))?;
                if size == 0 {
                    return Ok(());
                }
                let mut chunk = vec![0u8; size + 2];
                reader.read_exact(&mut chunk).map_err(Error::from)?;
                chunk.truncate(size);
                if !feed(&String::from_utf8_lossy(&chunk))? {
                    return Ok(());
                }
            }
        }
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).map_err(Error::from)? == 0 || !feed(&line)? {
                return Ok(());
            }
        }
    }

    /// Sends one request and turns a failure status into an error with the
    /// server's message.
    pub fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Answer> {
        let answer = self.send(method, path, body)?;
        if (200..300).contains(&answer.status) {
            return Ok(answer);
        }
        Err(failure_of(answer.status, &answer.body))
    }
}

/// A connection, plain or TLS.
trait Stream: Read + Write {}

impl<T: Read + Write> Stream for T {}

/// The error for a failed answer, with the server's message.
fn failure_of(status: u16, body: &[u8]) -> Error {
    let error: Option<ApiError> = serde_json::from_slice(body).ok();
    let message = error.as_ref().map_or_else(
        || format!("the server answered {status}"),
        |e| e.message.clone(),
    );
    match (status, error.map(|e| e.error)) {
        (401, _) | (_, Some(ErrorCode::Unauthorized)) => Error::Unauthorized(message),
        (404, _) | (_, Some(ErrorCode::NotFound)) => Error::NotFound(message),
        _ => Error::Refused(message),
    }
}

/// Server-sent events, line by line: an event ends at an empty line.
/// Comments (`:`) are skipped.
#[derive(Debug, Default)]
struct SseParser {
    event: String,
    data: String,
}

impl SseParser {
    fn line(&mut self, line: &str) -> Option<(String, String)> {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if self.data.is_empty() && self.event.is_empty() {
                return None;
            }
            let event = std::mem::take(&mut self.event);
            let data = std::mem::take(&mut self.data);
            return Some((
                if event.is_empty() {
                    "message".into()
                } else {
                    event
                },
                data,
            ));
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => self.event = value.to_string(),
            "data" => {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(value);
            }
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;

    #[test]
    fn urls_are_read() {
        assert_eq!(
            parse("https://samu.example.org").unwrap(),
            Target {
                tls: true,
                host: "samu.example.org".into(),
                port: 443
            }
        );
        assert_eq!(parse("http://127.0.0.1:8490/").unwrap().port, 8490);
        assert!(parse("samu.example.org").is_err());
        assert_eq!(
            parse("https://samu.example.org").unwrap().host_header(),
            "samu.example.org"
        );
        assert_eq!(
            parse("http://127.0.0.1:8490").unwrap().host_header(),
            "127.0.0.1:8490"
        );
        assert_eq!(parse("http://h:80").unwrap().host_header(), "h");
        assert!(parse("https://samu.example.org/api").is_err());
    }

    #[test]
    fn events_are_read_line_by_line() {
        let mut parser = SseParser::default();
        let mut events = Vec::new();
        for line in [
            ": connected\n",
            "\n",
            "event: changed\n",
            "id: 3\n",
            "data: {\"seq\":3}\n",
            "\n",
        ] {
            events.extend(parser.line(line));
        }
        assert_eq!(
            events,
            vec![("changed".to_string(), "{\"seq\":3}".to_string())]
        );
    }

    #[test]
    fn a_chunked_answer_is_put_together() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nsamu\r\n3\r\n ok\r\n0\r\n\r\n";
        let answer = split(raw).unwrap();
        assert_eq!(answer.status, 200);
        assert_eq!(answer.body, b"samu ok");
    }
}
