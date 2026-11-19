# Security

## Reporting a vulnerability

Please do not open a public issue for a security problem. Report it
privately through GitHub's "Report a vulnerability" button on the
repository's Security tab.

Helpful in a report: the version or commit, how the server is run, the
steps to reproduce, and what an attacker gains.

## What happens to a report

1. You get an answer within a week, saying whether the problem is
   confirmed.
2. The fix is made in the maintainer's checkout and is not pushed at
   once: every commit here is public with its message, so a fix that
   could be read before it runs is rolled out first and pushed after.
   If you want to check it first, you get it as a patch in the
   advisory's comments.
3. A release carries the fix.
4. The advisory is published right after the release, with a CVE if
   you like, and with your name if you want to be credited.

## Supported versions

Only the newest release gets security fixes; at present that is the
newest `0.1.x`. Fixes also go into `main`.

## Scope

samudo is meant to run inside a private network or VPN, behind a reverse
proxy with TLS (see [`docs/self-hosting.md`](docs/self-hosting.md)). It
has not been reviewed for exposure to the open internet. Reports about
the sign-in, sessions, agent tokens, account separation, the hooks and
calendar sources, and the Android app's local storage are especially
welcome.
