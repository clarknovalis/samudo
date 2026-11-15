#!/usr/bin/env bash
# Runs Hugo from the official image, so no local install is needed.
# The whole repository goes into the container: the site mounts ../docs
# and ../web/brand.
#   site/hugo.sh server          → http://localhost:1313
#   site/hugo.sh --minify --gc   → site/public/
set -euo pipefail

cd "$(dirname "$0")"
port=()
[[ "${1:-}" == server ]] && port=(-p 1313:1313) && set -- "$@" --bind 0.0.0.0
tty=(); [[ -t 0 ]] && tty=(-it)
exec docker run --rm "${tty[@]}" "${port[@]}" -u "$(id -u):$(id -g)" -e HOME=/tmp -e HUGO_CACHEDIR=/tmp/hugo_cache \
  -v "$(cd .. && pwd)":/repo -w /repo/site \
  ghcr.io/gohugoio/hugo:v0.168.0 "$@"
