---
number: 0134
title: One repository with its real history on GitHub
status: accepted
date: 2026-11-19
replaces: ["0108 §3, §6", "0113"]
---

# 0134 – One history on GitHub

## Context

The code was developed in a private repository and published as
snapshots: one commit per publication, with a message written for it
(0113). The reasons were private words in commit messages, which cannot
be taken back once pushed, and the fear of soiling the public repository.

Three things changed. Since the first publication no commit message has
carried a private word. The reviews miss the reasons for the design, and
a good part of those is in the commit messages, where measurements are
recorded. And the snapshot adds a step after every rollout.

## Decision

1. **GitHub is the only remote of the code**, with the real history,
   starting from the last snapshot. The history before it is not
   published; the old repository stays as a private archive.
2. **Two hooks guard what goes out**: `commit-msg` checks the message
   against the list of private words, `pre-push` runs the same check over
   the tree and gitleaks. The list itself is not published.
3. **The maintainer's operations stay in a private repository** of their
   own, as before.
4. **A private mirror pulls from GitHub**, as a backup. Nothing is pushed
   from the mirror.
5. **Agents do not push.** Only the maintainer does, also once the
   repository is public again. An agent commits, and names at its
   handover what is to be pushed. The `pre-push` hook of §2 holds the
   limits on the maintainer's side: only `main`, only forward, a
   release tag only on request.
6. **The repository is private during the changeover** and becomes
   public again when three things hold: the hooks have run on the real
   history for some days, the findings of the review that touch data
   integrity and the server's defaults are fixed and rolled out, and the
   40 most cited decision records are here (0133 §5).
7. **Before it becomes public again, §5 is decided anew**, from what the
   hooks stopped and what slipped past them while a push could still be
   corrected. Decided on 2026-11-20: see §5.

## Consequences

- Every commit is public with its time and its message. Messages are
  written for readers outside.
- A fix for a weakness can be read before it is rolled out. Where that
  matters, it is rolled out first and pushed after.
- The snapshot script and the `published/…` tags are no longer needed.
- Pull requests can be merged in the usual way.

## Changes

- 2026-11-19: The hosting plan enforces neither branch rules nor
  rulesets on a private repository, so the limits of §5 cannot be held
  by the server during the changeover. Until the repository is public
  again, agents do not push; only the maintainer does. The `pre-push`
  hook of §2 holds the same limits on the maintainer's side: only
  `main`, only forward, a release tag only on request. The agents'
  token and the rules on the server come with §7.
- 2026-11-19: The `pre-push` hook reads the added lines, the messages
  and the authors of every outgoing commit rather than the last tree,
  since a word that one commit brings and the next removes would stay
  in the history. Every refusal of a hook is counted, without the word.
- 2026-11-20: §5 decided anew, as §7 asks: agents do not push after the
  repository is public either. Until this day §5 allowed them to, with
  a token of their own that could write contents only, to `main` only,
  never forced, without tags, and only at a handover after the full
  check. That needs a second credential and rules on the server to
  hold it; one person pushing, behind the hooks, needs neither. The
  consequence about the hosting plan enforcing those limits is dropped
  with them.
