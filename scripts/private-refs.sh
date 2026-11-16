#!/usr/bin/env bash

# SPDX-FileCopyrightText: 2026 Marc Seibert
#
# SPDX-License-Identifier: AGPL-3.0-or-later

# Finds the maintainer's private names, hosts and paths in the files that
# are published (ADR 0108 §7). gitleaks finds keys; this finds what is not
# a secret but still not meant for the public tree.
#
#   scripts/private-refs.sh [PATTERNS]
#
# PATTERNS is a file of words, one per line, matched as whole words and
# ignoring case; else $SAMU_PRIVATE_REFS. It is not published itself. Without it the check fails rather than pass on
# nothing. Exit 0 when nothing is found, 1 with each hit on stdout, 2 when
# the patterns are missing or do not find themselves.
set -euo pipefail

cd "$(dirname "$0")/.."
patterns=${1:-${SAMU_PRIVATE_REFS:-}}
if [[ -z $patterns || ! -s $patterns ]]; then
    echo "private-refs: no patterns in ${patterns:-(none given: an argument or SAMU_PRIVATE_REFS)}" >&2
    exit 2
fi

# The words must find themselves: a check that passes on zero proves
# nothing (an empty or unreadable list, a grep that matches nothing).
words=$(grep -c -v '^[[:space:]]*$' "$patterns")
found=$(grep -c -i -w -F -f "$patterns" "$patterns" || true)
if [[ $found -ne $words ]]; then
    echo "private-refs: the patterns find $found of their own $words lines" >&2
    exit 2
fi

# Not checked: the vendored theme, which is someone else's.
private='/_vendor/'

hits=$(git ls-files -z | grep -z -v -E "$private" |
    xargs -0 grep -n -I -i -w -F -f "$patterns" -- 2>/dev/null || true)
if [[ -n $hits ]]; then
    printf '%s\n' "$hits"
    echo "private-refs: $(printf '%s\n' "$hits" | wc -l) private reference(s) in published files" >&2
    exit 1
fi
echo "private-refs: none in $(git ls-files | grep -c -v -E "$private") published files" >&2
