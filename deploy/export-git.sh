#!/bin/sh
# The git mirror of samu's export (ADR 0074 §9): run by samu-export.timer
# on the server. Writes each account as a directory of JSON (samu-server export)
# plus samu.org (samu export-org) into /srv/samu/export/<account>, commits
# when anything but the time in manifest.json changed, and pushes to every
# remote the repository has. The files of attachments (files/) stay out of
# git; they lie beside it and go into the server's backup with /srv/samu.
#
# The server knows nothing of git; without this script nothing is lost but
# the mirror.
#
#   SAMU_STACK             the compose stack, /srv/samu
#   SAMU_EXPORT_ACCOUNTS   the accounts, separated by spaces; else the line
#                          SAMU_EXPORT_ACCOUNTS= in the stack's .env
set -eu

STACK=${SAMU_STACK:-/srv/samu}
ACCOUNTS=${SAMU_EXPORT_ACCOUNTS:-$(sed -n 's/^SAMU_EXPORT_ACCOUNTS=//p' "$STACK/.env" 2>/dev/null || true)}
if [ -z "$ACCOUNTS" ]; then
    echo "export-git: no accounts; set SAMU_EXPORT_ACCOUNTS (or the line in $STACK/.env)" >&2
    exit 2
fi
OUT="$STACK/export"

# The container writes as 10010 (Dockerfile).
mkdir -p "$OUT"
chown 10010:10010 "$OUT"

compose() {
    docker compose --project-directory "$STACK" "$@"
}

status=0
for account in $ACCOUNTS; do
    dir="$OUT/$account"
    compose exec -T server samu-server export --account "$account" "/export/$account"
    compose exec -T server samu export-org "/export/$account"

    # Root runs git in a directory the container owns.
    g() { git -C "$dir" -c safe.directory="$dir" "$@"; }
    if [ ! -d "$dir/.git" ]; then
        g init -q -b main
        printf 'files/\n*.tmp\n' >"$dir/.gitignore"
    fi
    g add --all -- .
    # Only the time of the export changed: nothing to keep.
    if g diff --cached --quiet -- . ':(exclude)manifest.json'; then
        g reset -q
        continue
    fi
    g -c user.name="samu export" -c user.email="samu-export@localhost" \
        commit -q -m "samu export $account $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    for remote in $(g remote); do
        if ! g push -q "$remote" HEAD:main; then
            echo "export-git: push of $account to $remote failed" >&2
            status=1
        fi
    done
done
exit $status
