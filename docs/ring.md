# Capture from the Index ring

A gesture on the Pebble Index ring records a sentence; samudo files it as a
task in the inbox. There are two ways in. The usual one is a webhook from
the Pebble app to the server. The other goes through Tasker and the samudo
app on the same phone, and works offline.

## What lands in the inbox

Both ways make the same task:

- **Title:** the first line of the transcript, cut at a word boundary to at
  most 80 characters (`…` marks the cut). Without a transcript:
  `Aufnahme vom Ring, <date> <time>` in the account's time zone, so nothing
  is lost silently.
- **Note:** the whole transcript.
- **Created:** the time of the recording, not of the delivery.
- **Properties:** `source=index-ring`; through Tasker also `via=tasker`.
- **No quick capture:** words like `tomorrow` or `#tag` stay text. Speech
  rarely hits the shortcuts, and a misheard date would be worse than none.
- The audio is not kept; it stays in the Pebble app.

## Webhook from the Pebble app

The server takes deliveries at `POST /hooks/<name>` in the Pebble app's
format `index-v1`, signed with HMAC-SHA256. A request without a valid
signature, or older or newer than five minutes, gets `401` and makes
nothing.

1. Make a hook. Its secret is printed **once**:

   ```sh
   samu hook add ring
   ```

2. In the Pebble app, for the gesture you want (for example
   *Double click & hold*): URL `https://<your server>/hooks/ring`, mode
   *Both* or *Transcription only*, *Sign requests* on, and the secret from
   step 1.
3. *Send test event*: the server answers `200` and makes nothing.
4. Record something and look for it: `samu list --search <a word you said>`.

The Pebble app sends a delivery id with every request. The same id again
finds the task it already made instead of making a second one, so retries
are harmless.

Other commands:

```sh
samu hook list                  # the hooks of this account
samu hook secret ring           # a new secret; the old one stops at once
samu hook remove ring           # the tasks it made stay
samu hook send ring --secret "$SECRET" --text "Buy milk"   # try it without the ring
samu hook send ring --secret "$SECRET" --test              # a test event
```

## Offline through Tasker

The server is often reachable only through a VPN or the home network.
When the phone is outside both, the Pebble app's delivery fails. Tasker
can hand the text to the samudo app instead. The app files the task at once
in its outbox and sends it with the next sync.

In Tasker, add an action **Send Intent** to the task for the gesture
(see also [Android app](android.md#automation-with-tasker)):

| Field   | Value                                          |
|---------|------------------------------------------------|
| Action  | `de.samudo.app.action.RING`                    |
| Package | `de.samudo.app`                                |
| Target  | Broadcast Receiver                             |
| Extra   | `text:<variable with the transcript>`          |
| Extra   | `recorded_at:<Unix milliseconds>` (optional)   |

Without `recorded_at` the time of arrival is used. If nobody is signed in to
the app, or the task cannot be filed, a notification shows the text.

Try it without Tasker:

```sh
adb shell am broadcast -a de.samudo.app.action.RING -p de.samudo.app \
    --es text "'Buy milk'"
```

**Duplicates:** this path knows nothing of the webhook's delivery id. If
the same recording arrives both ways, it is in the inbox twice; the one
from Tasker carries `via=tasker`.

**Who may send:** any app on the phone can send this broadcast, because
Tasker cannot hold a permission of its own. It can only add to the inbox,
never read or change anything.
