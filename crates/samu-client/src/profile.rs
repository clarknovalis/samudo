// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later OR MIT OR Apache-2.0

//! Profiles: the accounts of one computer, each behind its own service on
//! the loopback (ADR 0086).
//!
//! They stand in the command line's configuration file, which the replica
//! and the local server read too (ADR 0086 §2): one place for a profile.
//!
//! ```toml
//! default = "privat"
//!
//! [profile.privat]                  # on a server, kept in step by the replica
//! account = "alice"
//! upstream = "https://samu.example.org"
//! port = 8490
//!
//! [profile.dienst]                  # only on this computer
//! account = "dienst"
//! port = 8491
//!
//! [front]                           # http://<profile>.samu.localhost
//! port = 80
//! ```
//!
//! In the browser each profile has the name `<profile>.samu.localhost`,
//! its own host and so its own cookie; with a front it needs no port
//! (ADR 0087).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// What is wrong with the profiles of a file.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("reading {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("reading {path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("no profile {0:?} in the configuration file")]
    Unknown(String),
    #[error("profile {name:?}: {message}")]
    Invalid { name: String, message: String },
    #[error("[front]: {0}")]
    Front(String),
}

/// One account on this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub account: String,
    /// The server it mirrors; none for an account only on this computer.
    pub upstream: Option<String>,
    /// Its service listens on `127.0.0.1:<port>`.
    pub port: u16,
}

impl Profile {
    /// Where its service answers.
    pub fn server(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Only on this computer, without a server it mirrors.
    pub fn is_local(&self) -> bool {
        self.upstream.is_none()
    }

    /// Its name in the browser (ADR 0087 §1).
    pub fn host(&self) -> String {
        format!("{}.samu.localhost", self.name.to_ascii_lowercase())
    }

    /// Its web UI: through the front without a port when there is one,
    /// else straight at its service, under its name either way
    /// (ADR 0087 §5).
    pub fn web(&self, front: Option<&Front>) -> String {
        match front {
            Some(front) if front.port == 80 => format!("http://{}", self.host()),
            Some(front) => format!("http://{}:{}", self.host(), front.port),
            None => self.web_direct(),
        }
    }

    /// Its web UI straight at its service, for when the front is away.
    pub fn web_direct(&self) -> String {
        format!("http://{}:{}", self.host(), self.port)
    }
}

/// The front on this computer, which takes the port out of a profile's
/// address (ADR 0087 §2).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Front {
    #[serde(default = "Front::default_port")]
    pub port: u16,
}

impl Front {
    fn default_port() -> u16 {
        80
    }
}

/// The profiles of a configuration file, and which one is the default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profiles {
    pub default: Option<String>,
    pub profiles: BTreeMap<String, Profile>,
    /// The front, if the file has a `[front]` section.
    pub front: Option<Front>,
}

#[derive(Debug, Default, Deserialize)]
struct File {
    default: Option<String>,
    #[serde(default)]
    profile: BTreeMap<String, Entry>,
    front: Option<Front>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    account: String,
    upstream: Option<String>,
    port: u16,
}

impl Profiles {
    /// The profiles in a file's text; everything else in it is left to
    /// whoever reads it.
    pub fn parse(text: &str, path: &Path) -> Result<Self, ProfileError> {
        let file: File = toml::from_str(text).map_err(|e| ProfileError::Parse {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
        let mut profiles = BTreeMap::new();
        let mut ports: BTreeMap<u16, String> = BTreeMap::new();
        let mut hosts: BTreeMap<String, String> = BTreeMap::new();
        for (name, entry) in file.profile {
            let invalid = |message: String| ProfileError::Invalid {
                name: name.clone(),
                message,
            };
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Err(invalid("a name of letters, digits, - and _ only".into()));
            }
            if entry.account.trim().is_empty() {
                return Err(invalid("no account".into()));
            }
            if entry.port == 0 {
                return Err(invalid("port 0".into()));
            }
            if let Some(other) = ports.insert(entry.port, name.clone()) {
                return Err(invalid(format!("port {} is also {other:?}'s", entry.port)));
            }
            // One name in the browser each (ADR 0087 §7).
            if let Some(other) = hosts.insert(name.to_ascii_lowercase(), name.clone()) {
                return Err(invalid(format!(
                    "the same name in the browser as {other:?}: names differing only in case"
                )));
            }
            if let Some(upstream) = &entry.upstream
                && !(upstream.starts_with("https://") || upstream.starts_with("http://"))
            {
                return Err(invalid(format!(
                    "upstream {upstream:?} is no http(s) address"
                )));
            }
            profiles.insert(
                name.clone(),
                Profile {
                    name,
                    account: entry.account,
                    upstream: entry.upstream,
                    port: entry.port,
                },
            );
        }
        if let Some(default) = &file.default
            && !profiles.contains_key(default)
        {
            return Err(ProfileError::Unknown(default.clone()));
        }
        if let Some(front) = &file.front
            && (front.port == 0 || ports.contains_key(&front.port))
        {
            return Err(ProfileError::Front(format!(
                "port {} is no free port for it",
                front.port
            )));
        }
        Ok(Self {
            default: file.default,
            profiles,
            front: file.front,
        })
    }

    /// The profiles of the configuration file; none without one.
    pub fn load() -> Result<Self, ProfileError> {
        match config_path() {
            Some(path) => Self::load_from(&path),
            None => Ok(Self::default()),
        }
    }

    /// The profiles of a file; none if it does not exist.
    pub fn load_from(path: &Path) -> Result<Self, ProfileError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text, path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(ProfileError::Read {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// A profile by name.
    pub fn get(&self, name: &str) -> Result<&Profile, ProfileError> {
        self.profiles
            .get(name)
            .ok_or_else(|| ProfileError::Unknown(name.to_string()))
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The configuration file: `$SAMU_CONFIG`, else
/// `$XDG_CONFIG_HOME/samu/config.toml`, else `~/.config/samu/config.toml`.
pub fn config_path() -> Option<PathBuf> {
    env("SAMU_CONFIG").map(PathBuf::from).or_else(|| {
        env("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| env("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|dir| dir.join("samu/config.toml"))
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;

    fn parse(text: &str) -> Result<Profiles, ProfileError> {
        Profiles::parse(text, Path::new("config.toml"))
    }

    #[test]
    fn online_and_local_profiles_next_to_the_rest_of_the_file() {
        let profiles = parse(
            r#"
            server = "https://samu.example"
            default = "privat"
            [open]
            jira = ["x", "{value}"]
            [profile.privat]
            account = "alice"
            upstream = "https://samu.example"
            port = 8490
            [profile.dienst]
            account = "work"
            port = 8491
            "#,
        )
        .unwrap();
        assert_eq!(profiles.default.as_deref(), Some("privat"));
        let privat = profiles.get("privat").unwrap();
        assert!(!privat.is_local());
        assert_eq!(privat.server(), "http://127.0.0.1:8490");
        let dienst = profiles.get("dienst").unwrap();
        assert!(dienst.is_local());
        assert_eq!(dienst.account, "work");
    }

    #[test]
    fn a_file_without_profiles_has_none() {
        let profiles = parse("server = \"https://samu.example\"\naccount = \"a\"\n").unwrap();
        assert_eq!(profiles, Profiles::default());
    }

    #[test]
    fn two_profiles_on_one_port_are_refused() {
        let err = parse(
            "[profile.a]\naccount = \"a\"\nport = 8490\n[profile.b]\naccount = \"b\"\nport = 8490\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("port 8490"), "{err}");
    }

    #[test]
    fn a_default_that_is_no_profile_is_refused() {
        let err =
            parse("default = \"x\"\n[profile.a]\naccount = \"a\"\nport = 8490\n").unwrap_err();
        assert!(matches!(err, ProfileError::Unknown(name) if name == "x"));
    }

    #[test]
    fn a_misspelt_key_is_refused_rather_than_ignored() {
        // `upstrem` ignored would make an online account a local one.
        let err = parse("[profile.a]\naccount = \"a\"\nport = 8490\nupstrem = \"https://x\"\n")
            .unwrap_err();
        assert!(matches!(err, ProfileError::Parse { .. }), "{err}");
    }

    #[test]
    fn a_name_that_cannot_be_a_file_name_is_refused() {
        let err = parse("[profile.\"a/b\"]\naccount = \"a\"\nport = 8490\n").unwrap_err();
        assert!(matches!(err, ProfileError::Invalid { .. }), "{err}");
    }

    #[test]
    fn a_profile_has_a_name_in_the_browser_with_or_without_a_front() {
        let file = "[profile.Privat]\naccount = \"a\"\nport = 8490\n";
        let profiles = parse(file).unwrap();
        let privat = profiles.get("Privat").unwrap();
        assert_eq!(profiles.front, None);
        assert_eq!(privat.host(), "privat.samu.localhost");
        assert_eq!(privat.web(None), "http://privat.samu.localhost:8490");
        let profiles = parse(&format!("{file}[front]\n")).unwrap();
        let front = profiles.front.as_ref();
        assert_eq!(front, Some(&Front { port: 80 }));
        assert_eq!(privat.web(front), "http://privat.samu.localhost");
        let profiles = parse(&format!("{file}[front]\nport = 8080\n")).unwrap();
        assert_eq!(
            privat.web(profiles.front.as_ref()),
            "http://privat.samu.localhost:8080"
        );
    }

    #[test]
    fn names_differing_only_in_case_are_refused() {
        let err = parse(
            "[profile.a]\naccount = \"a\"\nport = 8490\n[profile.A]\naccount = \"b\"\nport = 8491\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("only in case"), "{err}");
    }

    #[test]
    fn a_front_on_a_profile_port_is_refused() {
        let err =
            parse("[profile.a]\naccount = \"a\"\nport = 8490\n[front]\nport = 8490\n").unwrap_err();
        assert!(matches!(err, ProfileError::Front(_)), "{err}");
        let err = parse("[front]\nprot = 80\n").unwrap_err();
        assert!(matches!(err, ProfileError::Parse { .. }), "{err}");
    }

    #[test]
    fn an_unknown_profile_by_name() {
        assert!(matches!(
            Profiles::default().get("x"),
            Err(ProfileError::Unknown(_))
        ));
    }
}
