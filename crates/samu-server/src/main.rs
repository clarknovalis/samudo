// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The samu server: the single writer of every task (ADR 0004).

mod app;
mod blobs;
mod brake;
mod caldav;
mod calendar;
mod demo;
mod export;
mod front;
mod grant;
mod hooks;
mod import;
mod register;
mod replica;
mod web;
mod writer;

use std::future::IntoFuture;
use std::io::BufRead;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use jiff::Timestamp;
use samu_store::core::api::RegistrationMode;
use samu_store::{AccountStore, ServerStore};

/// The samu server.
#[derive(Debug, Parser)]
#[command(name = "samu-server", version = app::VERSION, about)]
struct Args {
    /// The data directory: server.db and accounts/<name>.db.
    #[arg(long, env = "SAMU_DATA", default_value = "/data", global = true)]
    data: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Serve the API.
    Serve {
        /// Where to listen. The boundary is the sign-in (ADR 0009 §3).
        #[arg(long, env = "SAMU_LISTEN", default_value = "0.0.0.0:8490")]
        listen: SocketAddr,
        /// Seals the sources of calendars (ADR 0047 §3): 32 bytes in Base64,
        /// like `openssl rand -base64 32`. Without it no calendar is fetched.
        #[arg(long, env = "SAMU_SECRET_KEY", hide_env_values = true)]
        secret_key: Option<String>,
        #[command(flatten)]
        limits: Limits,
    },
    /// The replica on a computer (ADR 0066): the same API and web UI on
    /// the loopback, from a mirror of the server, also while it is away.
    Replica {
        /// The server it mirrors.
        #[arg(long, env = "SAMU_UPSTREAM")]
        upstream: String,
        /// Where to listen; only a loopback address (ADR 0066 §1).
        #[arg(long, env = "SAMU_REPLICA_LISTEN", default_value = "127.0.0.1:8490")]
        listen: SocketAddr,
        /// The device it signs in to the server as (ADR 0067 §1); the host
        /// name without it.
        #[arg(long, env = "SAMU_DEVICE")]
        device: Option<String>,
        /// Its database and files; `$XDG_DATA_HOME/samu/replica` without it.
        #[arg(long, env = "SAMU_REPLICA_DIR")]
        dir: Option<PathBuf>,
    },
    /// Run a profile of the configuration file (ADR 0086 §2): with
    /// `upstream` the replica, without a server of its own; either way on
    /// 127.0.0.1 with the profile's port and its data in
    /// `$XDG_DATA_HOME/samu/<name>`. `--data` does not apply.
    Profile {
        name: String,
        #[command(subcommand)]
        command: Option<ProfileCommand>,
    },
    /// The front (ADR 0087 §2): on 127.0.0.1 and ::1 with the port of
    /// `[front]` in the configuration file, 80 without one, it hands each
    /// connection to the profile its `Host` names, so a profile is
    /// `http://<name>.samu.localhost`. `http://samu.localhost` lists them.
    /// Reads the file again when it changes.
    Front,
    /// Write an account as a directory of JSON files (ADR 0074 §5). Reads
    /// the database directly, also while the server runs.
    Export {
        #[arg(long)]
        account: String,
        /// Where to write; files that did not change are left alone.
        dir: PathBuf,
    },
    /// Make an account from an export again (ADR 0074 §6): only an empty
    /// one, made with `account add`, and only while the server is stopped.
    Restore {
        #[arg(long)]
        account: String,
        dir: PathBuf,
    },
    /// Manage accounts. Over HTTP only registration and deleting one's own
    /// (ADR 0099).
    #[command(subcommand)]
    Account(AccountCommand),
}

/// What a public instance limits (ADR 0097); without any, nothing more
/// than before.
#[derive(Debug, Clone, Default, clap::Args)]
struct Limits {
    /// Bytes per file, at most and by default 50 MB (ADR 0097 §2).
    #[arg(long, env = "SAMU_BLOB_MAX_BYTES")]
    blob_max_bytes: Option<u64>,
    /// Bytes of all files of an account together; no limit without it.
    #[arg(long, env = "SAMU_BLOB_QUOTA_BYTES")]
    blob_quota_bytes: Option<u64>,
    /// The hosts calendars are fetched from, separated by commas (ADR 0097
    /// §3); any without it.
    #[arg(long, env = "SAMU_CALENDAR_HOSTS", value_delimiter = ',')]
    calendar_hosts: Vec<String>,
    /// Accounts with a public password, separated by commas: no brake on
    /// their sign-ins per name (ADR 0098 §1).
    #[arg(long, env = "SAMU_BRAKE_EXEMPT", value_delimiter = ',')]
    brake_exempt: Vec<String>,
    /// A public demo, reset every so many minutes counted from midnight:
    /// the web UI shows a banner saying so (ADR 0098 §2).
    #[arg(long, env = "SAMU_DEMO_RESET_MINUTES", value_parser = clap::value_parser!(u32).range(1..=1440))]
    demo_reset_minutes: Option<u32>,
    /// Days after its last use a registered demo account is deleted
    /// (ADR 0099 §4); 7 without it.
    #[arg(long, env = "SAMU_DEMO_ACCOUNT_DAYS", value_parser = clap::value_parser!(u32).range(1..=3650))]
    demo_account_days: Option<u32>,
    /// Who may make an account over HTTP (ADR 0099 §1): `off` (only
    /// `samu-server account add`), `demo` (demo accounts that go after a
    /// rest) or `open` (ordinary accounts; strangers' data on the server).
    #[arg(long, env = "SAMU_REGISTRATION", default_value = "off", value_parser = ["off", "demo", "open"])]
    registration: String,
    /// Registered accounts at the same time (ADR 0099 §3); 100 without it.
    #[arg(long, env = "SAMU_REGISTRATION_MAX")]
    registration_max: Option<u64>,
}

impl Limits {
    fn blobs(&self) -> app::BlobLimits {
        app::BlobLimits {
            per_file: self.blob_max_bytes.unwrap_or(samu_store::MAX_BLOB_BYTES),
            per_account: self.blob_quota_bytes,
        }
    }

    fn calendar_hosts(&self) -> Option<Vec<String>> {
        let hosts: Vec<String> = self
            .calendar_hosts
            .iter()
            .map(|h| h.trim().to_ascii_lowercase())
            .filter(|h| !h.is_empty())
            .collect();
        (!hosts.is_empty()).then_some(hosts)
    }
}

#[derive(Debug, Subcommand)]
enum ProfileCommand {
    /// Manage the accounts of a local profile (ADR 0086 §3).
    #[command(subcommand)]
    Account(AccountCommand),
    /// Write a local profile's account as a directory of JSON files into
    /// `$XDG_DATA_HOME/samu/<name>/export` (ADR 0086 §10, ADR 0074 §5);
    /// `samu-export-profile` commits it to git.
    Export,
}

#[derive(Debug, Subcommand)]
enum AccountCommand {
    /// Create an account and its database.
    Add {
        name: String,
        /// Read the password from the first line of stdin.
        #[arg(long)]
        password_stdin: bool,
        /// Fill it with the demo content in this language (ADR 0099 §6).
        #[arg(long, value_enum)]
        demo: Option<demo::Lang>,
        /// With `--demo`: the demo's calendars and appointments on this
        /// CalDAV server, at `<caldav>/<name>/…`, signed in as the account
        /// with the password in SAMU_DEMO_CALDAV_PASSWORD. The sources are
        /// sealed with SAMU_SECRET_KEY, as the server needs it.
        #[arg(long, requires = "demo")]
        caldav: Option<String>,
    },
    /// Set a new password; ends the account's sessions.
    Passwd {
        name: String,
        #[arg(long)]
        password_stdin: bool,
    },
    /// Disable an account; ends its sessions.
    Disable { name: String },
    /// Delete an account with its database, files and sessions, for good
    /// (ADR 0099 §5). Refused while a running server holds the account:
    /// stop it first.
    Delete { name: String },
    /// List the accounts: name, state, created, how it came to be (cli,
    /// demo, open) and last sign-in.
    List,
}

/// A password from stdin, or typed without echo.
fn password(from_stdin: bool) -> Result<String> {
    if from_stdin {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        return Ok(line.trim_end_matches(['\r', '\n']).to_string());
    }
    let first = rpassword::prompt_password("password: ")?;
    let second = rpassword::prompt_password("again: ")?;
    if first != second {
        bail!("the two passwords differ");
    }
    Ok(first)
}

fn account(data: &std::path::Path, command: AccountCommand) -> Result<()> {
    std::fs::create_dir_all(data.join("accounts"))?;
    let mut server = ServerStore::open(&data.join("server.db"))?;
    match command {
        AccountCommand::Add {
            name,
            password_stdin,
            demo,
            caldav,
        } => {
            let password = password(password_stdin)?;
            // Checked before anything is made.
            let caldav = caldav
                .map(|base| -> Result<_> {
                    let password = std::env::var("SAMU_DEMO_CALDAV_PASSWORD")
                        .ok()
                        .filter(|p| !p.is_empty())
                        .context("--caldav needs SAMU_DEMO_CALDAV_PASSWORD")?;
                    let key = std::env::var("SAMU_SECRET_KEY")
                        .ok()
                        .filter(|k| !k.trim().is_empty())
                        .context("--caldav needs SAMU_SECRET_KEY to seal the sources")?;
                    Ok((demo::CalDav { base, password }, calendar::parse_key(&key)?))
                })
                .transpose()?;
            server.add_account(&name, &password, Timestamp::now())?;
            // Created and migrated now, so the first request finds it.
            let path = app::account_path(data, &name);
            let mut store = AccountStore::open(&path)?;
            eprintln!("account {name} created");
            if let Some(lang) = demo {
                let blobs = samu_store::BlobDir::beside(&path);
                let counts = demo::seed(&mut store, &blobs, lang)?;
                eprintln!("{name}: {counts}");
                if let Some((caldav, key)) = caldav {
                    let counts = demo::calendars(&mut store, &blobs, lang, &name, &caldav, &key)?;
                    eprintln!("{name}: {counts}");
                }
            }
        }
        AccountCommand::Passwd {
            name,
            password_stdin,
        } => {
            let password = password(password_stdin)?;
            server.set_password(&name, &password)?;
            eprintln!("password of {name} set; its sessions are ended");
        }
        AccountCommand::Disable { name } => {
            server.disable_account(&name, Timestamp::now())?;
            eprintln!("account {name} disabled; its sessions are ended");
        }
        AccountCommand::Delete { name } => {
            if !server.accounts()?.iter().any(|a| a.name == name) {
                bail!("there is no account {name:?}");
            }
            // Files first, under the account's lock: a running server that
            // holds it refuses the whole deletion, row included.
            AccountStore::remove(&app::account_path(data, &name))?;
            server.delete_account(&name)?;
            eprintln!("account {name} deleted, with its data and sessions");
        }
        AccountCommand::List => {
            for account in server.accounts()? {
                let state = if account.disabled {
                    "disabled"
                } else {
                    "active"
                };
                println!(
                    "{}\t{state}\t{}\t{}\t{}",
                    account.name,
                    account.created,
                    account.registered.as_str(),
                    account.last_active
                );
            }
        }
    }
    Ok(())
}

/// Runs a profile, or manages the accounts of a local one (ADR 0086).
fn profile(name: &str, command: Option<ProfileCommand>) -> Result<()> {
    let profiles = samu_client::profile::Profiles::load()?;
    let profile = profiles.get(name)?.clone();
    let dir = replica::data_home().join(&profile.name);
    let listen = SocketAddr::from(([127, 0, 0, 1], profile.port));
    // Its name in the browser, for the `Host` check (ADR 0087 §3).
    let host = profile.host();
    match (command, profile.upstream) {
        (Some(ProfileCommand::Account(command)), None) => account(&dir, command),
        (Some(ProfileCommand::Account(_)), Some(upstream)) => {
            bail!("profile {name} mirrors {upstream}: its accounts are managed there")
        }
        (Some(ProfileCommand::Export), None) => {
            export::export(&dir, &profile.account, &dir.join("export"))
        }
        (Some(ProfileCommand::Export), Some(upstream)) => {
            bail!("profile {name} mirrors {upstream}: it is kept there (ADR 0086 §10)")
        }
        (None, Some(upstream)) => tokio::runtime::Runtime::new()?.block_on(replica(
            upstream,
            listen,
            None,
            Some(dir),
            Some(host),
        )),
        (None, None) => {
            if !dir.join("server.db").exists() {
                eprintln!(
                    "profile {name} has no account yet: samu-server profile {name} account add {}",
                    profile.account
                );
            }
            std::fs::create_dir_all(dir.join("accounts"))
                .with_context(|| format!("creating {}", dir.display()))?;
            let secret_key = std::env::var("SAMU_SECRET_KEY").ok();
            if secret_key.as_deref().is_none_or(|k| k.trim().is_empty()) {
                // Where samu@.service reads it from.
                eprintln!(
                    "profile {name}: no SAMU_SECRET_KEY, its calendars are not fetched; \
                     set it in ~/.config/samu/{name}.env"
                );
            }
            tokio::runtime::Runtime::new()?.block_on(serve(
                dir,
                listen,
                secret_key,
                &Limits::default(),
                Some(Some(host)),
            ))
        }
    }
}

/// Serves the API; with `loopback` only requests to 127.0.0.1 or
/// localhost with its port, or to the profile's name in it, get through
/// (ADR 0086 §3, ADR 0087 §3).
async fn serve(
    data: PathBuf,
    listen: SocketAddr,
    secret_key: Option<String>,
    limits: &Limits,
    loopback: Option<Option<String>>,
) -> Result<()> {
    let key = secret_key
        .filter(|k| !k.trim().is_empty())
        .map(|k| calendar::parse_key(&k))
        .transpose()?;
    let without_key = key.is_none();
    let state = Arc::new(
        app::AppState::new(&data)?
            .with_secret_key(key)
            .with_blob_limits(limits.blobs())
            .with_calendar_hosts(limits.calendar_hosts())?
            .with_unbraked(&limits.brake_exempt)
            .with_demo_reset(limits.demo_reset_minutes)
            .with_demo_days(limits.demo_account_days)
            .with_registration(register::Registration::new(
                match limits.registration.as_str() {
                    "demo" => RegistrationMode::Demo,
                    "open" => RegistrationMode::Open,
                    _ => RegistrationMode::Off,
                },
                limits.registration_max,
            )),
    );
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .with_context(|| format!("listening on {listen}"))?;
    // The first line says where it listens; the CLI tests read it.
    eprintln!("samu-server {} on {}", app::VERSION, listener.local_addr()?);
    if without_key {
        calendar::say("no SAMU_SECRET_KEY: calendars are not fetched");
    }
    tokio::spawn(calendar::run(state.clone()));
    tokio::spawn(writer::run(state.clone()));
    tokio::spawn(app::close_idle_accounts(state.clone()));
    tokio::spawn(app::expire_demo_accounts(state.clone()));
    let mut router = app::router(state);
    if let Some(name) = loopback {
        let loopback = replica::Loopback {
            port: listener.local_addr()?.port(),
            name,
        };
        router = router.layer(axum::middleware::from_fn_with_state(
            loopback,
            replica::loopback_host,
        ));
    }
    run(listener, router).await
}

async fn replica(
    upstream: String,
    listen: SocketAddr,
    device: Option<String>,
    dir: Option<PathBuf>,
    name: Option<String>,
) -> Result<()> {
    if !listen.ip().is_loopback() {
        bail!("the replica listens only on the loopback, not on {listen} (ADR 0066 §1)");
    }
    let dir = dir.unwrap_or_else(replica::default_data);
    let device = device.unwrap_or_else(replica::default_device);
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .with_context(|| format!("listening on {listen}"))?;
    let address = listener.local_addr()?;
    let loopback = replica::Loopback {
        port: address.port(),
        name,
    };
    let state = replica::Replica::start(&dir, &upstream, &device, loopback)?;
    // The first line says where it listens; the tests read it.
    eprintln!(
        "samu-server {} replica of {upstream} on {address}, data in {}",
        app::VERSION,
        dir.display()
    );
    run(listener, replica::router(state)).await
}

/// How long requests may finish after Ctrl-C or SIGTERM. An `/events`
/// stream never finishes on its own; without a limit one open browser tab
/// (or the replica, which always listens) kept the process up until it
/// was killed.
const GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// Serves until Ctrl-C or SIGTERM, then lets requests finish for
/// [`GRACE`] at most.
async fn run(listener: tokio::net::TcpListener, router: axum::Router) -> Result<()> {
    let (stopping, mut stopped) = tokio::sync::watch::channel(false);
    let serve = axum::serve(listener, router).with_graceful_shutdown(async move {
        shutdown().await;
        let _ = stopping.send(true);
    });
    tokio::select! {
        served = serve.into_future() => served?,
        () = async {
            let _ = stopped.wait_for(|stop| *stop).await;
            tokio::time::sleep(GRACE).await;
        } => eprintln!("open connections after {}s: ending anyway", GRACE.as_secs()),
    }
    Ok(())
}

/// Ctrl-C or SIGTERM.
async fn shutdown() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let Ok(mut term) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        else {
            let _ = ctrl_c.await;
            return;
        };
        tokio::select! {
            _ = ctrl_c => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = ctrl_c.await;
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    match args.command {
        Command::Account(command) => account(&args.data, command),
        Command::Profile { name, command } => profile(&name, command),
        Command::Front => {
            let path = samu_client::profile::config_path()
                .context("no configuration file: neither SAMU_CONFIG nor HOME is set")?;
            tokio::runtime::Runtime::new()?.block_on(front::run(path))
        }
        Command::Export { account, dir } => export::export(&args.data, &account, &dir),
        Command::Restore { account, dir } => export::restore(&args.data, &account, &dir),
        Command::Serve {
            listen,
            secret_key,
            limits,
        } => tokio::runtime::Runtime::new()?
            .block_on(serve(args.data, listen, secret_key, &limits, None)),
        Command::Replica {
            upstream,
            listen,
            device,
            dir,
        } => tokio::runtime::Runtime::new()?.block_on(replica(upstream, listen, device, dir, None)),
    }
}

#[cfg(test)]
mod tests;
