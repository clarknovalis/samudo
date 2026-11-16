#!/bin/sh

# SPDX-FileCopyrightText: 2026 Marc Seibert
#
# SPDX-License-Identifier: AGPL-3.0-or-later

# The git mirror of a local profile (ADR 0086 §10), run by
# samu-export@<profile>.timer: writes the profile's account with
# `samu-server profile <profile> export` into
# ~/.local/share/samu/<profile>/export and commits there when anything but
# the time in manifest.json changed. No remote: a local profile's data do
# not leave this computer. Attachments (files/) lie beside the repository,
# outside git, as on the server (deploy/export-git.sh).
#
#   samu-export-profile <profile>
set -eu

profile=${1:?usage: samu-export-profile <profile>}
dir="${XDG_DATA_HOME:-$HOME/.local/share}/samu/$profile/export"

samu-server profile "$profile" export

g() { git -C "$dir" "$@"; }
if [ ! -d "$dir/.git" ]; then
    g init -q -b main
    printf 'files/\n*.tmp\n' >"$dir/.gitignore"
fi
g add --all -- .
# Only the time of the export changed: nothing to keep.
if g diff --cached --quiet -- . ':(exclude)manifest.json'; then
    g reset -q
    exit 0
fi
g -c user.name="samu export" -c user.email="samu-export@localhost" \
    commit -q -m "samu export $profile $(date -u +%Y-%m-%dT%H:%M:%SZ)"
