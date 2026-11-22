---
number: 0137
title: A published container image, built by GitHub Actions
status: accepted
date: 2026-11-20
adds_to: ["0009", "0118"]
---

# 0137 – A published container image

## Context

Self-hosting meant cloning the repository and building the image, a
release build of the whole server on the host that is to run it. That
takes a while and needs memory a small host may not have. A project
that is self-hosted is expected to publish an image.

The app is already built by GitHub when a release tag `v<version>` is
pushed (0118 §2, §5); the tag names the product version from
`Cargo.toml` (0118 §4). The maintainer's own server keeps being rolled
out from the workstation (0009). It does not depend on what is published.

## Decision

1. **An image on the GitHub Container Registry**,
   `ghcr.io/clarknovalis/samudo`, built from the `Dockerfile` in the
   repository (stage `compiled`). It holds `samu-server` and `samu`, the
   same as a local build.
2. **Two architectures, `linux/amd64` and `linux/arm64`**, each compiled
   on a runner of its own architecture, not under emulation. One
   manifest list names both.
3. **Tags.** A release tag `vX.Y.Z` publishes `X.Y.Z`, `X.Y` and
   `latest`, a pre-release such as `vX.Y.Z-rc.1` only its own version;
   the binaries report that version. Every commit on `main` whose
   CI passed publishes `edge` and `sha-<short>`; its binaries report
   `X.Y.Z+git<count>.<short>`, as the maintainer's rollout does.
4. **The release tag has to name the version in `Cargo.toml`**, or
   nothing is published, as for the app.
5. **Only commits that are on `main`.** The image for `edge` is built
   after the CI run of a push to `main` has passed, never for a pull
   request.
6. **Later, in this order, each with the same tag and version:** the
   binaries `samu` and `samu-server` and a `.deb` as assets of the GitHub
   release.
7. **The wire types on crates.io, as `samudo-api`**, with `[lib] name =
   "samu_api"`, so the code keeps its name, as does the crate inside the
   workspace. It has its own version, not the release's. The first
   version is uploaded by hand, as crates.io requires; after that a
   release tag publishes the version in its manifest through trusted
   publishing, without a stored token, if crates.io does not have it
   yet. A pre-release tag publishes none. A version there is permanent.

## Consequences

- `docker-compose.example.yml` takes the published image; building from
  the checkout stays as the second way, in a comment there and in
  `docs/self-hosting.md`.
- `latest` moves when the tag is pushed, while the app's release is
  still a draft until its APK is signed (0118 §2). The server needs no
  signature, so it does not wait.
- An `edge` build takes the time of two release builds on GitHub's
  runners for every push to `main`. A newer push cancels the build
  before it.
- The maintainer's server is still built on the workstation and rolled
  out privately; the published image is for everyone else.

## Changes

- 2026-11-22: §6 named the crate after the binaries and the `.deb` and
  left its name open; §7 now has it, ahead of them. The source has been
  public since the move to GitHub, but the registry the crate used to
  be published to is private.
