# Self-hosting

samudo is one binary, `samu-server`, with SQLite files in a data
directory. It serves the API, live events and the web UI on one port.
These steps use Docker Compose; the binary runs just as well under systemd.

## Before you start

- **Keep it private.** The boundary is the sign-in: without an active
  account everything except `/health` is closed, passwords are hashed
  with Argon2, sessions are tokens, and failed sign-ins are braked (see
  [A public instance](#a-public-instance)). It has not been reviewed for
  exposure to the internet. Run it inside
  a VPN or your home network, behind a reverse proxy that limits who may
  connect.
- **HTTPS.** The session cookie is marked `Secure`; browsers accept it
  over plain HTTP only on `localhost`. Put a reverse proxy with TLS in
  front (Caddy, nginx, Traefik).
- **Resources** are small: a few thousand tasks are computed in memory;
  half a CPU and 256 MB are plenty.

## The image

The image is published as `ghcr.io/clarknovalis/samudo`, for amd64 and
arm64:

- `latest` and a version like `0.1.5` (or `0.1` for its newest patch)
  come from a release;
- `edge` is the newest commit on `main` that passed its checks.

It is Debian slim with `samu-server` and `samu`, running as user 10010,
with its data in `/data`.

To build it yourself instead, from the repository:

```sh
git clone https://github.com/clarknovalis/samudo samu && cd samu
docker build -t samu-server .
```

## Compose

The repository has a ready file, [`docker-compose.example.yml`](../docker-compose.example.yml):
it takes the published image, binds the port to localhost for the
reverse proxy, and refuses to start without a secret key.

```sh
mkdir samu && cd samu
curl -fsSL -o docker-compose.yml \
  https://raw.githubusercontent.com/clarknovalis/samudo/main/docker-compose.example.yml
(umask 077; echo "SAMU_SECRET_KEY=$(openssl rand -base64 32)" > .env)
docker compose up -d
curl -s http://127.0.0.1:8490/health        # {"status":"ok",...}
```

To run your own build, replace the `image:` line with `build: .` in the
checkout and start with `docker compose up -d --build`.

A reverse proxy entry, for example with Caddy:

```
samu.example.org {
	reverse_proxy 127.0.0.1:8490
}
```

Live updates use server-sent events on `/events`; the proxy must not
buffer them (Caddy and nginx with `proxy_buffering off` are fine).

Behind the proxy the server needs `SAMU_TRUST_PROXY=true`, as the example
file sets it: only then does it believe `X-Forwarded-Host` and
`X-Forwarded-Proto`, and the pairing code for the Android app names
`https://samu.example.org`. Without it the code says `http://` and the
`Host` header, and a phone cannot pair through the proxy. Caddy sets both
headers by itself; nginx needs
`proxy_set_header X-Forwarded-Proto $scheme;` and
`proxy_set_header Host $host;`. Leave the option off when the server is
reached directly: then anyone could send these headers.

## Server options

| Option | Environment | Default | |
|---|---|---|---|
| `--data` | `SAMU_DATA` | `/data` | `server.db` (accounts, sessions) and `accounts/<name>.db` |
| `--listen` | `SAMU_LISTEN` | `0.0.0.0:8490` | address and port |
| `--secret-key` | `SAMU_SECRET_KEY` | none | seals calendar sources; see below |
| `--trust-proxy` | `SAMU_TRUST_PROXY` | off | believe `X-Forwarded-Host` and `X-Forwarded-Proto` for the address in a pairing code; only behind a reverse proxy that sets them |
| `--blob-max-bytes` | `SAMU_BLOB_MAX_BYTES` | 50 MB | largest attachment; never more than 50 MB |
| `--blob-quota-bytes` | `SAMU_BLOB_QUOTA_BYTES` | none | all attachments of one account together |
| `--calendar-hosts` | `SAMU_CALENDAR_HOSTS` | any | hosts calendars are fetched from, separated by commas |
| `--brake-exempt` | `SAMU_BRAKE_EXEMPT` | none | accounts with a public password, separated by commas: their sign-ins are not braked |
| `--demo-reset-minutes` | `SAMU_DEMO_RESET_MINUTES` | none | a demo reset every N minutes from midnight: the web UI shows a banner saying so |
| `--registration` | `SAMU_REGISTRATION` | `off` | who may make an account over HTTP: `off`, `demo` or `open`; see [Registration](#registration) |
| `--registration-max` | `SAMU_REGISTRATION_MAX` | 100 | registered accounts at the same time |
| `--demo-account-days` | `SAMU_DEMO_ACCOUNT_DAYS` | 7 | days after its last use a registered demo account is deleted |

## Accounts

Accounts are managed on the server; unless [registration](#registration)
is on, nobody makes one over HTTP:

```sh
docker compose exec -it server samu-server account add private
docker compose exec -it server samu-server account list
docker compose exec -it server samu-server account passwd private   # ends its sessions
docker compose exec server samu-server account disable private      # ends its sessions
```

Deleting an account removes its database, attachments and sessions for
good. A running server holds the account, so stop it first:

```sh
docker compose stop server
docker compose run --rm server account delete private
docker compose start server
```

To try samudo with something in it, make an account with the demo
content, in German or English: folders, projects and tasks, tags with two
workflows, habits with their past, perspectives, scopes and templates, its
dates counted from today. It goes only into a new account:

```sh
docker compose exec -it server samu-server account add try --demo en
```

With `--caldav <address>` the account also gets the demo's calendars and
appointments on a CalDAV server whose users are the accounts, as the
public demo does; the password is taken from `SAMU_DEMO_CALDAV_PASSWORD`,
and `SAMU_SECRET_KEY` must be set as for the server.

Each account is its own database. Use one for private life and one for
work rather than mixing them.

Then sign in from a client: the web UI at your address, the
[command line](cli.md#setup), or pair the [Android app](android.md).

## Registration

`SAMU_REGISTRATION` decides whether people make their own accounts, on the
sign-in page ("Konto anlegen") or with `samu register`:

- `off` (the default): only `samu-server account add`.
- `demo`: anybody can make a **demo account**. It always gets the demo
  content and is deleted with everything in it `SAMU_DEMO_ACCOUNT_DAYS`
  days (7) after it was last used; the web UI's banner names the day. The
  button "Ausprobieren" makes one without asking anything: the server picks
  a name and a password and shows them once.
- `open`: anybody can make an ordinary account, with the demo content if
  they like. Meant for a household or a club. **Strangers' data then lies
  on your server**: it is in your backups, you answer for it under data
  protection law, and only they can delete their account (or you, with
  `account delete`).

At most ten accounts are made per hour, and at most
`SAMU_REGISTRATION_MAX` (100) registered accounts exist at the same time;
accounts from `account add` do not count. A registered account can delete
itself ("Einstellungen > Konto" in the web UI, `samu account delete`); one
made with `account add` cannot.

Any account, registered or not, can take the demo content in afterwards
("Demodaten holen" in "Einstellungen > Konto", `samu demo en`): an empty
account gets all of it, one with content only a folder and a tag "Demo"
with everything below them, which go again by deleting the folder with its
tasks. Calendars come only with `account add --demo --caldav`.

## How far it scales

One `samu-server` process serves every account. Each account is a SQLite
file of its own under `accounts/`, opened on its first request; the
accounts and sessions are in `server.db`. Accounts do not wait for each
other when they write, and one account is one file to back up, restore or
delete.

That is made for one machine: a household, a club, a small team. What it
does not do:

- **One instance per data directory.** The server that opened an
  account holds its file; a second server on the same directory is
  refused every account the first one uses. So there is no load
  balancing and no failover: if the host is down, samu is down. Plan for
  it with [backups](#backup).
- **No sharing between accounts.** Each account sees only its own
  objects.

## The secret key

Calendar sources (subscription addresses, CalDAV passwords) are stored
encrypted in the account's database. The key is `SAMU_SECRET_KEY`, 32
bytes in Base64. Without it no calendar is fetched. Back it up **apart**
from the databases: whoever has both has the sources; whoever loses the key
sets each source again (`samu calendar source`).

## A public instance

A server anyone can reach should limit what a stranger can do with it:

- **Sign-ins** are always braked: after five failed sign-ins of one
  account name in 15 minutes, that name waits until the 15 minutes are
  over (HTTP 429); a good sign-in forgets the failures. At most two
  passwords are checked at once. Twenty wrong pairing codes in 15 minutes
  hold pairing the same way. A restart forgets all of it. An account
  whose password you publish (a demo) belongs in `SAMU_BRAKE_EXEMPT`, or
  anyone can lock it for everyone.
- **Attachments**: set `SAMU_BLOB_MAX_BYTES` and `SAMU_BLOB_QUOTA_BYTES`
  so the disk cannot fill up.
- **Calendars**: with `SAMU_SECRET_KEY` the server fetches whatever
  address a user gives it as a calendar, also inside your own network.
  `SAMU_CALENDAR_HOSTS=cal.example.org` limits that to the hosts you
  name; redirects to other hosts are not followed.
- **Registration**: leave it `off`, or use `demo`, whose accounts go by
  themselves. `open` on a public server collects strangers' data.

## Backup

Everything is in the volume: `server.db` and `accounts/<name>.db`, plus
the files of attachments. Copy them with the server stopped, or while it
runs with SQLite's backup:

```sh
sqlite3 /path/to/volume/accounts/private.db ".backup '/backup/private.db'"
```

Back up before every update that brings a database migration.

## Export and restore

An export is a plain directory you can read with a text editor, `jq` or
`git diff`, without samudo:

```sh
samu export ~/samu-export --org                                  # from a client, signed in
docker compose exec server samu-server export --account private /export/private   # on the server, also while it runs
```

It holds `manifest.json`, one JSON file per kind of object (`tasks.json`,
`projects.json`, `comments.json`, …), `logbook.json`, the files of
attachments under `files/<sha256>`, a `README.md` describing all of it,
and with `--org` a `samu.org` outline. The same state gives the same bytes,
so the directory works well in git: export on a timer and commit, and you
have a readable history. `deploy/export-git.sh` with the systemd units
`deploy/samu-export.service` and `.timer` is one way to do that (adjust the
paths and accounts at the top).

Not in an export: deleted objects, sessions and passwords, hooks, and the
sources of calendars.

To restore, stop the server, create an empty account and read the export
into it:

```sh
docker compose stop server
docker compose run --rm server account add private      # only if it does not exist
docker compose run --rm server restore --account private /export/private
docker compose start server
```

Then set each calendar source again.

## Updating

Back up, then `docker compose pull` and `docker compose up -d` (with
your own build: build the new image instead of pulling). The server
migrates the databases on start; a database migrated by a newer version
cannot be opened by an older one, so do not go back without the backup.
`/health` reports the running version.

## Webhooks

The server takes signed deliveries at `/hooks/<name>`, for example from a
smart ring. See [Capture from the Index ring](ring.md).
