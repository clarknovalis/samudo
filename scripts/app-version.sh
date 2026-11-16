#!/usr/bin/env bash

# SPDX-FileCopyrightText: 2026 Marc Seibert
#
# SPDX-License-Identifier: AGPL-3.0-or-later

# Prints the product version and the app's versionCode, separated by a
# space: `version` under [workspace.package] in a Cargo.toml, and
# major*10000 + minor*100 + patch with minor and patch below 100 (ADR 0118
# §4; android/app/build.gradle.kts computes the same).
#
#   scripts/app-version.sh [CARGO_TOML]    # default Cargo.toml; - reads stdin
#
# Exit 0 printed, 1 no version or not major.minor.patch.
set -euo pipefail

file=${1:-Cargo.toml}
[[ $file == - ]] && file=/dev/stdin
version=$(awk '
    /^\[/ { inside = ($0 == "[workspace.package]") }
    inside && /^version[ \t]*=/ { sub(/^[^"]*"/, ""); sub(/".*/, ""); print; exit }
' "$file")
[[ -n $version ]] || { echo "app-version: no version under [workspace.package]" >&2; exit 1; }
core=${version%%[-+]*}
if [[ ! $core =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]] ||
    ((10#${BASH_REMATCH[2]} >= 100 || 10#${BASH_REMATCH[3]} >= 100)); then
    echo "app-version: version $version: need major.minor.patch with minor and patch below 100" >&2
    exit 1
fi
echo "$version $((10#${BASH_REMATCH[1]} * 10000 + 10#${BASH_REMATCH[2]} * 100 + 10#${BASH_REMATCH[3]}))"
