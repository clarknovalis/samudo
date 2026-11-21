---
number: 0140
title: Voice capture – a transcript service, the assistant role, handing a recording on
status: accepted
date: 2026-11-21
adds_to: ["0010", "0013", "0039", "0063"]
---

# 0140 – Voice capture

## Context

A spoken task is the quickest capture on a phone: hold a key, talk,
done. What exists:

- The web UI records a voice note into the inbox: a task titled
  "Voice note <time>" with the recording as an attachment (0063). The
  app records audio as an attachment of a comment (0039 §6). Neither
  has a transcript; 0063 §3 says that a transcript, once there is one,
  replaces the title.
- The ring hook takes text that the ring's own app has transcribed
  (0010) and drops the audio.
- The app is a share target for text (0013). That was the agreed way
  for the sibling project catastr to turn a dictation into a task: it
  shares the transcript, samudo creates the task. In practice the
  transcript arrives minutes after the recording, and the recording
  then lives in the archive for good, although a spoken to-do is
  rarely worth keeping.
- catastr's app can hold Android's digital assistant role, so a long
  press of the side key opens its recorder. For a task that is a
  detour through another product, and someone who uses samudo alone
  has no such key.

Android gives the assistant role to one app at a time.

## Decision

1. **An optional transcript service, configured at the server.** One
   HTTP endpoint in the format of OpenAI's
   `POST /v1/audio/transcriptions`: multipart with `file`, `model` and
   optionally `language`; the answer is JSON with a top-level `text`.
   Settings: `SAMU_TRANSCRIBE_URL`, `SAMU_TRANSCRIBE_MODEL`,
   `SAMU_TRANSCRIBE_TOKEN_FILE` (sent as a bearer token, optional) and
   `SAMU_TRANSCRIBE_LANGUAGE` (optional, otherwise the service
   detects it). Without a URL nothing changes and no audio leaves the
   server. samudo ships no speech model and computes nothing itself.
   - Self-hosted services that speak this format: speaches, and the
     server of whisper.cpp when started with
     `--inference-path /v1/audio/transcriptions --convert`.
     `--convert` (ffmpeg) is needed there because the app records AAC
     in MPEG-4 and the web UI Opus in WebM.
   - The server, not the app, calls the service, so that web, app and
     CLI share one way and no client holds a token for it.
2. **The transcript is a field of the attachment**, `transcript`
   (text, absent until there is one). It is shown under the player
   and found by the search. This is a new version of `samu-api` with a
   fixture, and a migration.
3. **`POST /attachments/{id}/transcribe`** with a session asks for the
   transcript of one audio attachment; CLI
   `samu transcribe <attachment>`. The answer comes at once; the
   server works in the background, because a self-hosted model may
   need longer than the recording lasts.
   - While it works, the attachment says so in a field
     `transcript_state` (`pending`, `failed`); a failure keeps its
     reason, and asking again tries again. Pending requests are picked
     up again after a restart.
   - A client asks the same by creating the attachment with
     `transcript_state` `pending`. That request travels like any
     change, through an outbox too, so a recording made offline is
     transcribed once it has arrived. A server without a service drops
     it.
   - `503` when no service is configured. The session tells clients
     whether one is, so that they offer the action only then.
   - An empty answer (nothing was said) is a result, not a failure:
     the transcript is the empty text.
4. **A voice note** is a task that was created from a recording (web
   button, the app's recorder, shared audio). Its capture asks for
   the transcript itself and marks the task with the property
   `source=voice`. When the text arrives and nobody has changed the
   title since the task was made, the first line becomes the title, cut at a
   word boundary to at most 80 characters, and the whole text the
   note if the note is still empty (as 0010 §3). A title or note that
   someone has changed in the meantime stays. The capture parser does
   not run on spoken text (0010 §8). Without a service a voice note
   stays what it is today.
5. **The recording stays** as the attachment of its task and leaves
   with it. A transcript can be wrong, and the recording is the only
   way to check.
6. **A recorder in the app**: one view with the elapsed time, stop,
   discard and "hand on" (§8). It starts recording when it opens.
   Stop creates the voice note in the inbox without a further
   question, as 0063 §2; the scope fills in what it prescribes for
   new tasks. Offline the recording waits in the blob outbox (0039
   §5) and the transcript is requested once it has been uploaded. It
   may appear over the lock screen and shows nothing else of the app
   there. Ways in: a button beside quick capture, an app shortcut,
   and §7.
7. **The app can be the digital assistant.** An `ACTION_ASSIST`
   filter on the recorder; who chooses samudo under "Default apps →
   Digital assistant" opens the recorder with a long press of the
   side key. Nothing else of an assistant is implemented.
8. **Handing a recording on.** "Hand on" in the recorder stops and
   sends the file with `ACTION_SEND` to an app the user picks, and
   samudo keeps nothing of it. In the other direction the app
   becomes a share target for `audio/*`: shared audio becomes a voice
   note, exactly as if it had been recorded here.
   - This is how the two products combine for someone who uses both:
     whichever holds the assistant role, the other is one tap away,
     and a to-do never has to be stored in the archive. It is an
     intent between apps on the phone; neither product learns the
     other's API or holds a token for it (0003 §7 stays).
   - catastr's recorder needs the same button on its side. That is a
     task there, not part of this record.
9. **Not built: an import of recordings from catastr.** Whether a
   recording is a task is known while speaking; an import would
   decide it afterwards and leave every to-do in the archive. A
   recording that turns out to be a task later goes the way of 0013,
   as text with its link. A voice note that is worth keeping gets
   there from samudo's side: catastr's connector reads the tasks that
   carry a tag chosen for it, with their attachments.
10. **The ring** keeps the text of its own app (0010). Sending its
    audio through this service is possible later and not part of this
    record.

## Consequences

- A migration and a new `samu-api` version: this is built on a feature
  branch.
- With a service configured, recordings are sent to it. Self-hosting
  documentation has to say so plainly, with the two self-hosted
  services as the first examples. The public demo runs without one.
- The transcript of a long recording counts against nothing; the
  recording itself is bound by the limits for blobs.
- Web: the voice note button asks for the transcript after the upload;
  the list shows the placeholder title until the text is there.
- The assistant role is exclusive on Android. Who gives it to samudo
  loses the previous assistant on that key.

## Not now

- An audio attachment of a comment gets its transcript on request
  only, with a button beside the player, not automatically.
- A quick-settings tile for the recorder: later, if the assistant key
  and the shortcut are not enough.

## Changes

- 2026-11-21, while building: §3 names the second way to ask (an
  attachment created with `pending`), §4 the property that marks a
  voice note and "not changed since" instead of "still the
  placeholder", which the server cannot tell in every language. §9
  names catastr's connector.
