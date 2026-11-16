# Profiles: accounts on a computer, offline

A computer can run samudo itself, on the loopback, one service per
account. Each is a **profile** in `~/.config/samu/config.toml`:

- **with `upstream`**, the profile is a **replica** of your server: it
  keeps a mirror and an outbox, so the web UI, the command line and Emacs
  keep working while the server is out of reach, and catch up when it is
  back;
- **without `upstream`**, it is a **local server**: the account and its
  data never leave the computer (for example a work account that must stay
  on the work laptop).

```toml
default = "private"

[profile.private]                  # mirrors the server
account = "private"
upstream = "https://samu.example.org"
port = 8490

[profile.work]                     # only on this computer
account = "work"
port = 8491

[front]                            # optional: http://<profile>.samu.localhost
port = 80
```

The command line, Emacs and the desktop widget take the `default`
profile, or the one named with `--profile` / `SAMU_PROFILE`.

## Setting up

```sh
make install                                  # samu
make install-replica                          # samu-server and the user unit samu@.service
samu-server profile work account add work     # a local profile needs its account
systemctl --user enable --now samu@private samu@work
samu profile list                             # running? signed in?
samu --profile work login
```

Each profile runs as `samu-server profile <name>` on `127.0.0.1` with its
port, its data in `~/.local/share/samu/<name>`; logs with
`journalctl --user -u samu@<name>`.

A new `samu-server` binary needs `systemctl --user restart samu@<name>`: a
profile that keeps running keeps the old binary, and with it the old schema,
since migrations run when the server starts. New routes then answer 404 and
the export stops at `schema N, this build at M`.

## Signing in to a replica

Sign in as usual. The replica passes account and password on to the
server; when that succeeds it keeps the server's session for itself and
gives the client a session of its own, which works offline too. The first
sign-in on a new computer needs a connection. What you write through the
replica carries the replica's device on the server.

Agents do not sign in at a replica; they talk to the server directly
(`--server https://samu.example.org`).

## In the browser

Each profile has its own host name, `http://<profile>.samu.localhost:<port>`,
and so its own cookie: several accounts are signed in side by side.
Browsers resolve `*.localhost` to the loopback by themselves.

Without ports, through the **front**: a small system service on port 80
that hands each connection to the profile its host name names.

```sh
make install-front               # needs sudo: /etc/systemd/system/samu-front.service
```

Then `http://private.samu.localhost`, `http://work.samu.localhost`, and
`http://samu.localhost` lists the profiles. A new profile is picked up
without a restart; a new `samu-server` binary needs
`sudo systemctl restart samu-front`.

## Calendars of a local profile

A local profile fetches its calendars itself and needs its own secret key
in `~/.config/samu/<profile>.env`, which `samu@.service` reads:

```sh
(umask 077; echo "SAMU_SECRET_KEY=$(openssl rand -base64 32)" > ~/.config/samu/work.env)
systemctl --user restart samu@work
```

Never replace that key: a new one cannot open the stored sources. A
replica fetches no calendars; that is the server's job.

## Backing up a local profile

A local profile has no server behind it, so back it up on the computer:
`samu-export@.timer` exports it every 15 minutes into a git repository
without a remote under `~/.local/share/samu/<profile>/export` (attachments
beside it, not in git).

```sh
systemctl --user enable --now samu-export@work.timer
git -C ~/.local/share/samu/work/export log --oneline | head
```
