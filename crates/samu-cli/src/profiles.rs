// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `samu profile`: the accounts of this computer (ADR 0086 §5).

use crate::client::Client;
use anyhow::Result;
use clap::Subcommand;
use serde_json::json;

use crate::config::Config;
use crate::output::{Format, emit};

#[derive(Debug, Subcommand)]
pub(crate) enum ProfileCommand {
    /// The profiles of the configuration file: account, online or local,
    /// port, its web UI, whether its service answers and whether this
    /// command line is signed in there.
    List,
}

pub(crate) fn run(config: &Config, format: Format, command: ProfileCommand) -> Result<()> {
    match command {
        ProfileCommand::List => {
            let profiles = Config::profiles()?;
            let mut records = Vec::new();
            for profile in profiles.profiles.values() {
                if profile.name.contains('_') {
                    eprintln!(
                        "profile {}: _ is no letter of a host name; browsers take it, but - is safer (ADR 0087 §7)",
                        profile.name
                    );
                }
                let answers = Client::new(&profile.server(), None)
                    .and_then(|c| c.call("GET", "/health", None))
                    .is_ok();
                let signed_in = config
                    .state
                    .join(format!("{}.token", profile.name))
                    .exists();
                records.push(json!({
                    "name": profile.name,
                    "default": profiles.default.as_deref() == Some(profile.name.as_str()),
                    "account": profile.account,
                    "kind": if profile.is_local() { "local" } else { "online" },
                    "upstream": profile.upstream,
                    "port": profile.port,
                    "server": profile.server(),
                    "web": profile.web(profiles.front.as_ref()),
                    "running": answers,
                    "signed_in": signed_in,
                }));
            }
            emit(
                format,
                &records,
                &[
                    "name",
                    "account",
                    "kind",
                    "port",
                    "web",
                    "running",
                    "signed_in",
                    "default",
                ],
            )
        }
    }
}
