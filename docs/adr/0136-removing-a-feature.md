---
number: 0136
title: A feature leaves in three stages
status: accepted
date: 2026-11-19
adds_to: ["0004", "0015"]
---

# 0136 – Removing a feature

## Context

There is a rule for the client that is newer than the server: an
operation the server does not know is rejected (0015). There is none for
the other direction. A phone that was offline may send, weeks later,
changes to a field that no longer exists; the server would reject them,
and nothing says what the client does with a change that is rejected for
good. Removing a field is also a breaking change of `samu-api`, which
makes taking a feature out expensive.

## Decision

1. **Deprecated.** The field or module is marked in the table of modules
   (0135) and in the API documentation. The interfaces no longer offer
   it; values that exist are still shown.
2. **Retired.** The server accepts a change to it, drops it and answers
   "accepted, ignored", so that every old outbox drains instead of
   getting stuck. It counts, per device, what still arrives.
3. **Removed**, at the earliest 90 days after retiring and when the
   count has been silent: migration, new fixtures, new version of
   `samu-api`.
4. **An experimental module (0135 §4)** may be retired without having
   been deprecated, and its removal is not a major version.
5. **A client does not repeat a rejected change without end.** A test
   holds that.
6. **Migrations stay as they are until 1.0.** They are folded into one
   base schema then, with the rule that an installation must have been on
   a named version before it updates.

## Consequences

- `samu-api` gets the outcome "ignored", with a fixture.
- The 90 days are longer than a session lives without use (30 days): a
  device that was away longer signs in again anyway.

## Changes

None yet.
