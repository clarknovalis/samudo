// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where the server is, which account, which device, and where the tokens
//! live (ADR 0019 §7).
//!
//! Flags win over the environment, the environment over the file. The file
//! is `$SAMU_CONFIG`, else `$XDG_CONFIG_HOME/samu/config.toml`, else
//! `~/.config/samu/config.toml`. It may hold profiles (ADR 0086), see
//! [`samu_client::profile`]; a profile chosen with `--profile` or
//! `SAMU_PROFILE` gives server and account, only flags go before it. The
//! file's `default` profile applies when neither a profile nor a server
//! (`--server`, `SAMU_SERVER`) is named, so an agent that names a server
//! keeps talking to it (ADR 0067 §5).
//!
//! ```toml
//! server = "https://samu.example.org"
//! account = "privat"
//! device = "laptop"
//! # Where `samu:<uuid>` opens in the browser (ADR 0075 §2); `server`
//! # without it. With the replica: "http://127.0.0.1:8490".
//! web = "http://127.0.0.1:8490"
//!
//! # What opens a property's value on this device (ADR 0054 §5): argv,
//! # no shell, {value} put in as it is.
//! [open]
//! jira = ["mein-script", "{value}"]
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use samu_client::profile::{Profile, Profiles, config_path};
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
struct File {
    server: Option<String>,
    account: Option<String>,
    device: Option<String>,
    web: Option<String>,
    #[serde(default)]
    open: BTreeMap<String, Vec<String>>,
}

/// The settings one invocation runs with.
#[derive(Debug, Clone)]
pub(crate) struct Config {
    pub(crate) server: Option<String>,
    pub(crate) account: Option<String>,
    pub(crate) device: String,
    /// Where tokens are kept.
    pub(crate) state: PathBuf,
    /// The web UI a `samu:` address opens in (ADR 0075 §2).
    pub(crate) web: Option<String>,
    /// With a front: its port and the profile's address without it, for
    /// when the front does not answer (ADR 0087 §5).
    web_fallback: Option<(u16, String)>,
    /// The profile it runs with (ADR 0086 §5), if any.
    pub(crate) profile: Option<Profile>,
    /// The file's server outside any profile, for taking over its token.
    plain_server: Option<String>,
    /// Per property key, the command that opens its value (ADR 0054 §5).
    pub(crate) open: BTreeMap<String, Vec<String>>,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn home() -> Option<PathBuf> {
    env("HOME").map(PathBuf::from)
}

fn state_dir() -> Result<PathBuf> {
    env("SAMU_STATE")
        .map(PathBuf::from)
        .or_else(|| {
            env("XDG_STATE_HOME")
                .map(PathBuf::from)
                .or_else(|| home().map(|h| h.join(".local/state")))
                .map(|dir| dir.join("samu"))
        })
        .ok_or_else(|| anyhow!("no place for the session token: set HOME or SAMU_STATE"))
}

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "cli".into())
}

impl Config {
    /// Reads the file and the environment, then applies the flags.
    pub(crate) fn load(
        server: Option<String>,
        account: Option<String>,
        profile: Option<String>,
    ) -> Result<Self> {
        let (file, profiles) = match config_path() {
            Some(path) if path.exists() => {
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?;
                let file: File =
                    toml::from_str(&text).with_context(|| format!("reading {}", path.display()))?;
                (file, Profiles::parse(&text, &path)?)
            }
            _ => (File::default(), Profiles::default()),
        };
        let chosen = profile.or_else(|| env("SAMU_PROFILE"));
        let (server, account, profile) = match chosen {
            Some(name) => {
                let p = profiles.get(&name)?.clone();
                (
                    Some(server.unwrap_or_else(|| p.server())),
                    Some(account.unwrap_or_else(|| p.account.clone())),
                    Some(p),
                )
            }
            None => {
                let server = server.or_else(|| env("SAMU_SERVER"));
                let account = account.or_else(|| env("SAMU_ACCOUNT"));
                match (&server, &profiles.default) {
                    (None, Some(name)) => {
                        let p = profiles.get(name)?.clone();
                        (
                            Some(p.server()),
                            Some(account.unwrap_or_else(|| p.account.clone())),
                            Some(p),
                        )
                    }
                    _ => (
                        server.or(file.server.clone()),
                        account.or(file.account),
                        None,
                    ),
                }
            }
        };
        let web = env("SAMU_WEB").or_else(|| match &profile {
            Some(p) => Some(p.web(profiles.front.as_ref())),
            None => file.web,
        });
        let web_fallback = match (env("SAMU_WEB"), &profile, &profiles.front) {
            (None, Some(p), Some(front)) => Some((front.port, p.web_direct())),
            _ => None,
        };
        Ok(Self {
            server,
            account,
            device: env("SAMU_DEVICE").or(file.device).unwrap_or_else(hostname),
            state: state_dir()?,
            web,
            web_fallback,
            open: file.open,
            profile,
            plain_server: file.server,
        })
    }

    /// The profiles of the configuration file (ADR 0086).
    pub(crate) fn profiles() -> Result<Profiles> {
        Ok(Profiles::load()?)
    }

    pub(crate) fn server(&self) -> Result<&str> {
        self.server.as_deref().ok_or_else(|| {
            anyhow!("no server: use --server, SAMU_SERVER, or `server` in the config file")
        })
    }

    /// The web UI: `web`, else the server. A profile's address through
    /// the front only while the front answers, else straight at its port
    /// (ADR 0087 §5).
    pub(crate) fn web(&self) -> Result<String> {
        if let Some((port, direct)) = &self.web_fallback {
            let front = std::net::SocketAddr::from(([127, 0, 0, 1], *port));
            let wait = std::time::Duration::from_millis(200);
            if std::net::TcpStream::connect_timeout(&front, wait).is_err() {
                return Ok(direct.clone());
            }
        }
        match self.web.as_deref() {
            Some(web) => Ok(web.to_string()),
            None => self.server().map(str::to_string),
        }
    }

    pub(crate) fn account(&self) -> Result<&str> {
        self.account.as_deref().ok_or_else(|| {
            anyhow!("no account: use --account, SAMU_ACCOUNT, or `account` in the config file")
        })
    }

    /// What names the files of this account here: the profile, else the
    /// account (ADR 0086 §5).
    fn key(&self) -> Result<String> {
        match &self.profile {
            Some(p) => Ok(p.name.clone()),
            None => Ok(self.account()?.to_string()),
        }
    }

    /// The settings for another account on the same server, as for one
    /// just registered; none under a profile of another account, whose
    /// token file it would take over.
    pub(crate) fn for_account(&self, name: &str) -> Option<Self> {
        match &self.profile {
            Some(p) if p.account != name => None,
            _ => {
                let mut config = self.clone();
                config.account = Some(name.to_string());
                Some(config)
            }
        }
    }

    /// The file `<key>.<ext>` in the state directory. A profile's file that
    /// is not there yet is taken over once from the account's, if that one
    /// belonged to the same server and account (ADR 0086 §5).
    fn state_file(&self, ext: &str) -> Result<PathBuf> {
        let path = self.state.join(format!("{}.{ext}", self.key()?));
        if let Some(p) = &self.profile
            && !path.exists()
            && self.plain_server.as_deref() == Some(p.server().as_str())
            && self.account.as_deref() == Some(p.account.as_str())
        {
            let old = self.state.join(format!("{}.{ext}", p.account));
            if old.exists() {
                std::fs::copy(&old, &path)
                    .with_context(|| format!("taking over {}", old.display()))?;
            }
        }
        Ok(path)
    }

    /// The token file of the account.
    pub(crate) fn token_path(&self) -> Result<PathBuf> {
        self.state_file("token")
    }

    /// The saved token, if signed in.
    pub(crate) fn token(&self) -> Result<Option<String>> {
        let path = self.token_path()?;
        match std::fs::read_to_string(&path) {
            Ok(token) => Ok(Some(token.trim().to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Keeps a token, readable only by the user.
    pub(crate) fn save_token(&self, token: &str) -> Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::create_dir_all(&self.state)
            .with_context(|| format!("creating {}", self.state.display()))?;
        let path = self.token_path()?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .with_context(|| format!("writing {}", path.display()))?;
        file.write_all(token.as_bytes())?;
        Ok(())
    }

    pub(crate) fn forget_token(&self) -> Result<()> {
        match std::fs::remove_file(self.token_path()?) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    /// Where the scope chosen on this device is kept (ADR 0035 §4): next to
    /// the token, one per account.
    fn scope_path(&self) -> Result<PathBuf> {
        self.state_file("scope")
    }

    /// Whether this device follows the scopes' schedule (ADR 0043 §7):
    /// the file holds `auto`.
    pub(crate) fn follows_schedule(&self) -> Result<bool> {
        let path = self.scope_path()?;
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(text.trim() == "auto"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Follow the schedule on this device; a choice by name or `--none`
    /// ends it.
    pub(crate) fn follow_schedule(&self) -> Result<()> {
        let path = self.scope_path()?;
        std::fs::create_dir_all(&self.state)
            .with_context(|| format!("creating {}", self.state.display()))?;
        std::fs::write(&path, "auto").with_context(|| format!("writing {}", path.display()))
    }

    /// The scope chosen on this device by name, if any; none while it
    /// follows the schedule.
    pub(crate) fn chosen_scope(&self) -> Result<Option<samu_api::Id>> {
        let path = self.scope_path()?;
        match std::fs::read_to_string(&path) {
            Ok(text) if text.trim() == "auto" => Ok(None),
            Ok(text) => {
                Ok(Some(text.trim().parse().with_context(|| {
                    format!("{} holds no scope id", path.display())
                })?))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Chooses a scope on this device, or none.
    pub(crate) fn choose_scope(&self, scope: Option<samu_api::Id>) -> Result<()> {
        let path = self.scope_path()?;
        match scope {
            Some(id) => {
                std::fs::create_dir_all(&self.state)
                    .with_context(|| format!("creating {}", self.state.display()))?;
                std::fs::write(&path, id.to_string())
                    .with_context(|| format!("writing {}", path.display()))
            }
            None => match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                _ => Ok(()),
            },
        }
    }
}
