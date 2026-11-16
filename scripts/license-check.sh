#!/usr/bin/env bash
# The crates that may also be used under MIT (samu-api, samu-client) must
# not depend on a crate of this workspace that is only AGPL: otherwise
# their MIT label would be void. Dev-dependencies do not ship and are not
# checked. Prints how many dependencies it looked at, so a check of none
# shows as such.
set -euo pipefail

cargo metadata --format-version 1 --no-deps --offline | jq -r '
  (.packages | map({key: .name, value: .license}) | from_entries) as $lic
  | [ .packages[] | select(.license | test("MIT")) | .name as $n
      | .dependencies[] | select(.path != null and .kind != "dev")
      | {from: $n, to: .name, ok: (($lic[.name] // "") | test("MIT"))} ] as $deps
  | ($deps[] | select(.ok | not)
     | "FAIL \(.from) may be used under MIT but depends on \(.to) (\($lic[.to]))"),
    "checked \($deps | length) workspace dependencies of MIT crates"
' > "${TMPDIR:-/tmp}/license-check.$$"
trap 'rm -f "${TMPDIR:-/tmp}/license-check.$$"' EXIT

sed 's/^/license-check: /' "${TMPDIR:-/tmp}/license-check.$$" >&2
if grep -q '^FAIL' "${TMPDIR:-/tmp}/license-check.$$"; then
  exit 1
fi
