# syntax=docker/dockerfile:1.7

# SPDX-FileCopyrightText: 2026 Marc Seibert
#
# SPDX-License-Identifier: AGPL-3.0-or-later

# samu server (ADR 0009).
#
# Two ways to the binaries: `compiled` builds them here, `prebuilt` takes
# what a release script built on the workstation in dist/bin/. Prebuilt
# fits only a workstation on Debian 13 like the runtime below (same glibc);
# for anything else, `compiled` is the way. The ARG has to come before the first stage, or
# `FROM ${BINARIES}` does not resolve.
ARG BINARIES=compiled

FROM rust:1.93-slim-trixie AS compiled
# libsqlite3-sys compiles SQLite's C source; nothing else needs a compiler.
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
# samu-server embeds the web UI (include_str! in crates/samu-server/src/web.rs).
COPY web ./web
ARG SAMU_VERSION
ENV SAMU_VERSION=${SAMU_VERSION}
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/build/target,sharing=locked \
    cargo build --release --locked --bin samu-server --bin samu \
    && mkdir -p /out \
    && cp target/release/samu-server target/release/samu /out/

FROM scratch AS prebuilt
COPY dist/bin/ /out/

FROM ${BINARIES} AS source

FROM debian:trixie-slim AS runtime
# tzdata: jiff looks up the account's zone (ADR 0007 §2, ADR 0017) in the
# system database; without it no zone is known and every date is wrong.
# ca-certificates for whatever the server later reaches out to (catastr).
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/*

COPY --from=source /out/samu-server /out/samu /usr/local/bin/

# An unprivileged user with a fixed id, so volume permissions are the same on
# every host. 10010 to stay clear of catastr's 10001–10004.
RUN groupadd --gid 10010 samu \
    && useradd --uid 10010 --gid 10010 --home-dir /data --no-create-home samu \
    && mkdir -p /data \
    && chown -R 10010:10010 /data

USER 10010:10010
WORKDIR /data
VOLUME ["/data"]
ENV SAMU_DATA=/data SAMU_LISTEN=0.0.0.0:8490
EXPOSE 8490

ENTRYPOINT ["samu-server"]
CMD ["serve"]
