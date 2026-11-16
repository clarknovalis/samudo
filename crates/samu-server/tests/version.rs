// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The binary starts and reports its version.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

use std::process::Command;

#[test]
fn version_prints_the_package_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_samu-server"))
        .arg("--version")
        .output()
        .expect("run samu-server");
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    assert_eq!(
        stdout.trim(),
        format!("samu-server {}", env!("CARGO_PKG_VERSION"))
    );
}
