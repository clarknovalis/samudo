# Security

## Reporting a vulnerability

Please do not open a public issue for a security problem. Report it
privately through GitHub's "Report a vulnerability" button on the
repository's Security tab. You will get an answer within a week; a fix
and its disclosure are coordinated with you.

Helpful in a report: the version or commit, how the server is run, the
steps to reproduce, and what an attacker gains.

## Supported versions

There is no release yet. Fixes go into `main`.

## Scope

samudo is meant to run inside a private network or VPN, behind a reverse
proxy with TLS (see [`docs/self-hosting.md`](docs/self-hosting.md)). It
has not been reviewed for exposure to the open internet. Reports about
the sign-in, sessions, agent tokens, account separation, the hooks and
calendar sources, and the Android app's local storage are especially
welcome.
