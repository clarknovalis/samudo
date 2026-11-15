//! The front (ADR 0087 §2): on port 80 of the loopback it hands each
//! connection to the service of the profile its `Host` names, so a
//! profile is `http://<profile>.samu.localhost` in the browser.
//!
//! It works on the connection, not on the request: it reads the head up
//! to `Host`, connects to the profile's port, writes what it read and then
//! copies bytes both ways. `/events`, uploads and whatever comes later go
//! through without an HTTP client of its own. A browser keeps an HTTP/1.1
//! connection to one host; a later request on it naming another is
//! refused by the service itself (ADR 0087 §3).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use samu_client::profile::{Front, Profiles};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// The most a request head may take before `Host`.
const HEAD_LIMIT: usize = 16 * 1024;
/// How long a client may take to send its head.
const HEAD_TIME: Duration = Duration::from_secs(10);
/// The name that lists the profiles.
const OVERVIEW: &str = "samu.localhost";

/// The configuration file, read again when it changes.
struct Config {
    path: PathBuf,
    kept: Mutex<(Option<SystemTime>, Profiles)>,
}

impl Config {
    fn load(path: PathBuf) -> Result<Self> {
        let profiles = Profiles::load_from(&path)?;
        let modified = modified(&path);
        Ok(Self {
            path,
            kept: Mutex::new((modified, profiles)),
        })
    }

    /// The profiles as the file has them now; the last good ones while
    /// it does not read.
    fn profiles(&self) -> Profiles {
        let mut kept = match self.kept.lock() {
            Ok(kept) => kept,
            Err(poisoned) => poisoned.into_inner(),
        };
        let now = modified(&self.path);
        if now != kept.0 {
            match Profiles::load_from(&self.path) {
                Ok(profiles) => {
                    eprintln!("{} read again", self.path.display());
                    *kept = (now, profiles);
                }
                Err(e) => eprintln!("{e}; keeping the profiles read before"),
            }
        }
        kept.1.clone()
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Runs the front on `127.0.0.1` and `::1` with the port of `[front]`,
/// 80 without one, until Ctrl-C or SIGTERM.
pub(crate) async fn run(path: PathBuf) -> Result<()> {
    let config = Arc::new(Config::load(path)?);
    let port = config.profiles().front.map_or(80, |f: Front| f.port);
    let mut listeners = vec![
        TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port)))
            .await
            .with_context(|| format!("listening on 127.0.0.1:{port}"))?,
    ];
    // Browsers try ::1 for *.localhost too; a computer without IPv6 does
    // without it.
    match TcpListener::bind(SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, port))).await {
        Ok(listener) => listeners.push(listener),
        Err(e) => eprintln!("not on [::1]:{port}: {e}"),
    }
    let addresses: Vec<String> = listeners
        .iter()
        .filter_map(|l| l.local_addr().ok())
        .map(|a| a.to_string())
        .collect();
    // The first line says where it listens; the tests read it.
    eprintln!(
        "samu-server {} front on {}, profiles from {}",
        crate::app::VERSION,
        addresses.join(" and "),
        config.path.display()
    );
    let mut tasks = tokio::task::JoinSet::new();
    for listener in listeners {
        tasks.spawn(accept(listener, config.clone()));
    }
    tokio::select! {
        () = crate::shutdown() => {}
        Some(ended) = tasks.join_next() => ended??,
    }
    Ok(())
}

async fn accept(listener: TcpListener, config: Arc<Config>) -> Result<()> {
    loop {
        let (stream, _) = listener.accept().await?;
        let config = config.clone();
        tokio::spawn(async move {
            if let Err(e) = connection(stream, &config).await {
                eprintln!("{e:#}");
            }
        });
    }
}

/// One connection: to the profile its `Host` names, or the overview, or
/// an answer why not.
async fn connection(mut client: TcpStream, config: &Config) -> Result<()> {
    let head = match tokio::time::timeout(HEAD_TIME, read_head(&mut client)).await {
        Ok(head) => head?,
        Err(_) => return answer(&mut client, 408, "Request Timeout", "too slow").await,
    };
    let Some(head) = head else {
        return answer(&mut client, 400, "Bad Request", "no request head").await;
    };
    let Some(host) = host_of(&head) else {
        return answer(&mut client, 400, "Bad Request", "no Host").await;
    };
    let profiles = config.profiles();
    if host == OVERVIEW {
        return page(&mut client, &profiles).await;
    }
    let Some(profile) = profiles.profiles.values().find(|p| p.host() == host) else {
        let text = format!("no profile is {host}; http://{OVERVIEW} lists them");
        return answer(&mut client, 421, "Misdirected Request", &text).await;
    };
    let mut service = match TcpStream::connect(("127.0.0.1", profile.port)).await {
        Ok(service) => service,
        Err(e) => {
            let text = format!(
                "profile {} does not answer on 127.0.0.1:{} ({e}): systemctl --user start samu@{}",
                profile.name, profile.port, profile.name
            );
            return answer(&mut client, 502, "Bad Gateway", &text).await;
        }
    };
    service.write_all(&head).await?;
    // Either side ending ends the connection; that is no error.
    let _ = tokio::io::copy_bidirectional(&mut client, &mut service).await;
    Ok(())
}

/// What came in up to the end of the request head, or `None` if the
/// connection ended or the head grew too long first.
async fn read_head(stream: &mut TcpStream) -> Result<Option<Vec<u8>>> {
    let mut head = Vec::with_capacity(1024);
    let mut chunk = [0u8; 4096];
    loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(None);
        }
        head.extend_from_slice(&chunk[..read]);
        if head.windows(4).any(|w| w == b"\r\n\r\n") {
            return Ok(Some(head));
        }
        if head.len() > HEAD_LIMIT {
            return Ok(None);
        }
    }
}

/// The host a request head names, lower case and without its port.
fn host_of(head: &[u8]) -> Option<String> {
    let end = head.windows(4).position(|w| w == b"\r\n\r\n")?;
    let text = std::str::from_utf8(&head[..end]).ok()?;
    let value = text.split("\r\n").skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("host")
            .then(|| value.trim())
    })?;
    let name = match value.rsplit_once(':') {
        Some((name, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => value,
    };
    (!name.is_empty()).then(|| name.to_ascii_lowercase())
}

async fn answer(client: &mut TcpStream, code: u16, reason: &str, text: &str) -> Result<()> {
    respond(client, code, reason, "text/plain", &format!("{text}\n")).await
}

async fn respond(
    client: &mut TcpStream,
    code: u16,
    reason: &str,
    kind: &str,
    body: &str,
) -> Result<()> {
    let head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {kind}; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    client.write_all(head.as_bytes()).await?;
    client.write_all(body.as_bytes()).await?;
    client.shutdown().await?;
    Ok(())
}

/// `http://samu.localhost`: the profiles, as links.
async fn page(client: &mut TcpStream, profiles: &Profiles) -> Result<()> {
    respond(client, 200, "OK", "text/html", &overview(profiles)).await
}

fn overview(profiles: &Profiles) -> String {
    let front = profiles.front.clone().unwrap_or(Front { port: 80 });
    let rows: String = profiles
        .profiles
        .values()
        .map(|p| {
            let web = p.web(Some(&front));
            let kind = match &p.upstream {
                Some(upstream) => format!("online, {}", escape(upstream)),
                None => "lokal".to_string(),
            };
            format!(
                "<li><a href=\"{}\">{}</a> – {}, {kind}</li>\n",
                escape(&web),
                escape(&p.name),
                escape(&p.account)
            )
        })
        .collect();
    let rows = if rows.is_empty() {
        "<li>keine Profile in config.toml</li>\n".to_string()
    } else {
        rows
    };
    format!(
        "<!doctype html>\n<html lang=\"de\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width\">\
         <title>samudo – Profile</title>\
         <style>body{{font:16px/1.5 system-ui,sans-serif;margin:2rem auto;max-width:36rem;padding:0 1rem}}</style>\
         </head><body>\n<h1>samudo – Profile</h1>\n<ul>\n{rows}</ul>\n</body></html>\n"
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;

    #[test]
    fn the_host_without_its_port_in_lower_case() {
        let head = |host: &str| format!("GET / HTTP/1.1\r\nAccept: */*\r\n{host}\r\n\r\n");
        assert_eq!(
            host_of(head("Host: Privat.samu.localhost").as_bytes()).as_deref(),
            Some("privat.samu.localhost")
        );
        assert_eq!(
            host_of(head("host:privat.samu.localhost:8080").as_bytes()).as_deref(),
            Some("privat.samu.localhost")
        );
        assert_eq!(host_of(head("X-Host: a").as_bytes()), None);
        assert_eq!(host_of(head("Host: ").as_bytes()), None);
        assert_eq!(
            host_of(b"GET / HTTP/1.1\r\nHost: a"),
            None,
            "no end of head"
        );
    }

    #[test]
    fn the_overview_links_every_profile_escaped() {
        let profiles = Profiles::parse(
            "[profile.privat]\naccount = \"a<b\"\nupstream = \"https://samu.example\"\n\
             port = 8490\n[profile.dienst]\naccount = \"work\"\nport = 8491\n",
            Path::new("config.toml"),
        )
        .unwrap();
        let page = overview(&profiles);
        assert!(
            page.contains("href=\"http://privat.samu.localhost\""),
            "{page}"
        );
        assert!(
            page.contains("href=\"http://dienst.samu.localhost\""),
            "{page}"
        );
        assert!(page.contains("a&lt;b, online"), "{page}");
        assert!(page.contains("work, lokal"), "{page}");
    }
}
