#!/usr/bin/env bash
# Runs Hugo from the official image, so no local install is needed.
# The whole repository goes into the container: the page mounts ../site (theme, screenshots)
# and ../web/brand.
#   www/hugo.sh server           → http://localhost:1314
#   www/hugo.sh --minify --gc    → www/public/
set -euo pipefail

cd "$(dirname "$0")"
port=()
[[ "${1:-}" == server ]] && port=(-p 1314:1314) && set -- "$@" --bind 0.0.0.0 --port 1314
tty=(); [[ -t 0 ]] && tty=(-it)
exec docker run --rm "${tty[@]}" "${port[@]}" -u "$(id -u):$(id -g)" -e HOME=/tmp -e HUGO_CACHEDIR=/tmp/hugo_cache \
  -v "$(cd .. && pwd)":/repo -w /repo/www \
  ghcr.io/gohugoio/hugo:v0.168.0 "$@"
