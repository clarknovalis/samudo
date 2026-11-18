# HTTP API

Every client of samudo, the web UI included, uses this API and nothing
else. The [command line](cli.md) covers all of it, so `samu <command>
--format json` is the quickest way to see a response. The wire types are
the Rust crate `samu-api` (serde only, versioned on its own); JSON
examples of every type are in
[`crates/samu-api/tests/fixtures/`](../crates/samu-api/tests/fixtures/).

## Conventions

- Ids are UUIDv7, timestamps ISO 8601 in UTC with `Z`, lengths of time
  ISO 8601 durations (`PT15M`, `P2D`).
- A date is `{"date": "2026-11-20"}` (a whole day, no zone) or
  `{"at": "2026-11-20T09:00:00+01:00[Europe/Berlin]"}` (an instant with
  its zone).
- Fields a reader does not know are kept; enum values from a newer version
  read as `unknown` and are never written back.
- Errors are a status code with
  `{"error": "not_found", "message": "…"}`; kinds include `unauthorized`,
  `not_found`, `offline` (a replica without its server).

## Signing in

```http
POST /login
{"account": "private", "password": "…", "device": "laptop"}
```

answers with a session `token` and its expiry, and sets a cookie for
browsers. Send the token as `Authorization: Bearer <token>`. With
`"kind": "agent"` the session is an agent's: what it writes is marked as
such. `GET /session` shows the session and the account's zone and
settings; `POST /logout` ends it.

Pairing: `POST /pair/code` (with the password again) makes a one-time code
for ten minutes; `POST /pair` trades it for a session on the new device.

## Reading

| Route | |
|---|---|
| `GET /available` | open tasks with availability, every reason, state and next state; filters: available only, today, project, all, scope, perspective; `elsewhere=true` adds what another own scope alone shows, marked `elsewhere` with that scope, for a hint only |
| `GET /blocks?perspective=<id>` | what a perspective shows, block by block: `[{title, icon, block, missing, tasks}]`, `tasks` as in `/available`; a plain perspective is one block, and `/available` answers a block perspective with every block's tasks once; with `scope` |
| `GET /available/projects`, `/available/folders` | projects (stuck, review due) and folders; projects take `elsewhere=true` as well |
| `GET /forecast` | the forecast sections, each row with why it is there |
| `GET /objects/{id}` | one object of any kind |
| `GET /objects/{id}/logbook` | its history |
| `GET /logbook?since=&until=&kind=&device=&limit=` | the account's logbook, newest first, each entry with `object_kind` and `title`; the last 7 days and 200 entries by default |
| `GET /changes?after=<seq>` | every object changed after a change number, for a client's cache |
| `GET /habits`, `/habits/{id}` | habit status, streaks, entries |
| `GET /appointments` | appointments of some days, repetitions unfolded |
| `GET /alarms` | the reminders of tasks and appointments that go off in a window of at most seven days, by time |
| `GET /clocks/report`, `GET /report` | time and the report |
| `GET /scopes/scheduled` | which scope the schedules choose now, and until when |
| `GET /blobs/{sha256}` | the file behind an attachment |
| `GET /export` | the whole account (see [Self-hosting](self-hosting.md#export-and-restore)) |
| `GET /health` | `{"status": "ok", "version": …, "api": …}`, without a session |

## Writing

Nearly every write is a list of **changes** to `POST /changes`. One change
is one of:

```json
{"op": "create", "object": "<id>", "kind": "task", "value": {…whole object…}, "hlc": …}
{"op": "set",    "object": "<id>", "field": "due", "value": {"date": "2026-11-20"}, "hlc": …}
{"op": "add",    "object": "<id>", "field": "tags", "element": "<tag id>", "position": 0, "hlc": …}
{"op": "remove", "object": "<id>", "field": "tags", "element": "<tag id>", "hlc": …}
{"op": "delete", "object": "<id>", "hlc": …}
```

`set` with `null` clears a field; a property is the field
`properties.<key>`. `create` is idempotent by id, so a client makes the id
itself and may send the same change twice. Changes to states and
completion may carry a `note` for the logbook.

`hlc` is a hybrid logical clock stamp, `{"wall": <ms>, "counter": n,
"device": "laptop"}`. The server decides **per field**: the later stamp
wins. Each change gets an outcome:

```json
{"outcome": "applied", "seq": 185}
{"outcome": "superseded", "by": {"wall": …, "counter": 0, "device": "phone"}}
{"outcome": "rejected", "reason": "field \"due\" expects {\"date\"} or {\"at\"}"}
```

Writes with logic of their own have their own routes:

| Route | |
|---|---|
| `POST /capture` | a line of [quick capture](quick-capture.md): preview, create, or apply to a task |
| `POST /objects/{id}/delete` | delete with what hangs on it |
| `PUT /blobs/{sha256}` | upload a file; then create an attachment that names it |
| `POST /render` | a note or comment as a document tree |
| `PUT /calendars/{id}/source`, `POST /calendars/{id}/refresh`, `POST /calendars/discover` | calendar sources; a source may take user and password from another calendar on the same host (`credentials_from`) |
| `POST /calendars/{id}/discover` | the other calendars on the server of a CalDAV calendar, with its stored user and password |
| `POST /calendars/{id}/import` | an iCalendar file (`text/calendar`) into a writable CalDAV calendar |
| `POST /appointments/{id}/edit` | change an appointment or occurrences of one |
| `GET`, `POST /hooks` | webhooks of the account |
| `POST /hooks/{name}` | a signed delivery from outside, no session |

## Live

`GET /events` is a stream of server-sent events. A `changed` event carries
ids only, never content:

```json
{"seq": 184, "gone": ["…"], "added": ["…"], "changed": ["…"], "config": false}
```

A client loads what it shows again; `config: true` means the account's
settings changed. A `reset` event (`{"seq": 190}`) means: load everything
you show from scratch.

## Web UI routes

`/` and `/lib/…` serve the web UI from the binary; `/o/{id}` opens an
object in it.
