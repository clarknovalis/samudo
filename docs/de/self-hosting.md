# Selbst hosten

samudo ist ein Binary, `samu-server`, mit SQLite-Dateien in einem
Datenverzeichnis. Es liefert API, Live-Ereignisse und Web-Oberfläche auf
einem Port aus. Diese Schritte nehmen Docker Compose; das Binary läuft
genauso gut unter systemd.

## Bevor es losgeht

- **Privat halten.** Die Grenze ist die Anmeldung: Ohne aktives Konto ist
  alles außer `/health` zu, Passwörter werden mit Argon2 gehasht,
  Sitzungen sind Tokens. Mehr nicht. Es ist nicht darauf geprüft, offen im
  Internet zu stehen. Am besten in einem VPN oder im Heimnetz betreiben,
  hinter einem Reverse-Proxy, der begrenzt, wer verbinden darf.
- **HTTPS.** Das Sitzungs-Cookie ist `Secure`; Browser nehmen es über
  einfaches HTTP nur auf `localhost` an. Einen Reverse-Proxy mit TLS
  davorsetzen (Caddy, nginx, Traefik).
- **Ressourcen** sind klein: Ein paar tausend Aufgaben werden im Speicher
  berechnet; eine halbe CPU und 256 MB reichen reichlich.

## Das Image

Das Image ist als `ghcr.io/clarknovalis/samudo` veröffentlicht, für amd64
und arm64:

- `latest` und eine Version wie `0.1.5` (oder `0.1` für deren neuesten
  Patch) kommen aus einem Release;
- `edge` ist der neueste Commit auf `main`, der seine Prüfungen bestanden
  hat.

Es ist Debian slim mit `samu-server` und `samu`, läuft als Benutzer 10010,
mit seinen Daten in `/data`.

Wer es lieber selbst baut, aus dem Repository:

```sh
git clone https://github.com/clarknovalis/samudo samu && cd samu
docker build -t samu-server .
```

## Compose

Das Repository hat eine fertige Datei, [`docker-compose.example.yml`](../../docker-compose.example.yml):
sie nimmt das veröffentlichte Image, bindet den Port an localhost für den
Reverse-Proxy und startet nicht ohne geheimen Schlüssel.

```sh
mkdir samu && cd samu
curl -fsSL -o docker-compose.yml \
  https://raw.githubusercontent.com/clarknovalis/samudo/main/docker-compose.example.yml
(umask 077; echo "SAMU_SECRET_KEY=$(openssl rand -base64 32)" > .env)
docker compose up -d
curl -s http://127.0.0.1:8490/health        # {"status":"ok",...}
```

Für einen eigenen Build im Checkout die Zeile `image:` durch `build: .`
ersetzen und mit `docker compose up -d --build` starten.

Ein Eintrag im Reverse-Proxy, zum Beispiel mit Caddy:

```
samu.example.org {
	reverse_proxy 127.0.0.1:8490
}
```

Live-Aktualisierungen nutzen Server-Sent Events auf `/events`; der Proxy
darf sie nicht puffern (Caddy, und nginx mit `proxy_buffering off`, sind
in Ordnung).

Hinter dem Proxy braucht der Server `SAMU_TRUST_PROXY=true`, wie es die
Beispieldatei setzt: Nur dann glaubt er `X-Forwarded-Host` und
`X-Forwarded-Proto`, und der Kopplungscode für die Android-App nennt
`https://samu.example.org`. Ohne die Option steht im Code `http://` und der
`Host`-Header, und ein Telefon lässt sich über den Proxy nicht koppeln.
Caddy setzt beide Header von sich aus; nginx braucht
`proxy_set_header X-Forwarded-Proto $scheme;` und
`proxy_set_header Host $host;`. Die Option bleibt aus, wenn der Server
direkt erreicht wird: Dann könnte jeder diese Header mitschicken.

## Optionen des Servers

| Option | Umgebung | Vorgabe | |
|---|---|---|---|
| `--data` | `SAMU_DATA` | `/data` | `server.db` (Konten, Sitzungen) und `accounts/<name>.db` |
| `--listen` | `SAMU_LISTEN` | `0.0.0.0:8490` | Adresse und Port |
| `--secret-key` | `SAMU_SECRET_KEY` | keiner | versiegelt die Kalenderquellen; siehe unten |
| `--trust-proxy` | `SAMU_TRUST_PROXY` | aus | glaubt `X-Forwarded-Host` und `X-Forwarded-Proto` für die Adresse im Kopplungscode; nur hinter einem Reverse-Proxy, der sie setzt |
| `--blob-max-bytes` | `SAMU_BLOB_MAX_BYTES` | 50 MB | größter Anhang; nie mehr als 50 MB |
| `--blob-quota-bytes` | `SAMU_BLOB_QUOTA_BYTES` | keine | alle Anhänge eines Kontos zusammen |
| `--calendar-hosts` | `SAMU_CALENDAR_HOSTS` | alle | Hosts, von denen Kalender abgerufen werden, durch Kommas getrennt |
| `--brake-exempt` | `SAMU_BRAKE_EXEMPT` | keine | Konten mit öffentlichem Passwort, durch Kommas getrennt: ihre Anmeldungen werden nicht gebremst |
| `--demo-reset-minutes` | `SAMU_DEMO_RESET_MINUTES` | keiner | alle N Minuten ab Mitternacht ein Reset der Demo: die Web-Oberfläche zeigt ein Banner dazu |
| `--registration` | `SAMU_REGISTRATION` | `off` | wer über HTTP ein Konto anlegen darf: `off`, `demo` oder `open`; siehe [Registrierung](#registrierung) |
| `--registration-max` | `SAMU_REGISTRATION_MAX` | 100 | registrierte Konten gleichzeitig |
| `--demo-account-days` | `SAMU_DEMO_ACCOUNT_DAYS` | 7 | Tage nach der letzten Benutzung, nach denen ein registriertes Demo-Konto gelöscht wird |
| `--transcribe-url` | `SAMU_TRANSCRIBE_URL` | keine | wohin Aufnahmen für ein Transkript geschickt werden; siehe [Transkripte](#transkripte) |
| `--transcribe-model` | `SAMU_TRANSCRIBE_MODEL` | `whisper-1` | das Modell, nach dem der Dienst gefragt wird |
| `--transcribe-token-file` | `SAMU_TRANSCRIBE_TOKEN_FILE` | keine | eine Datei mit dem Bearer-Token des Dienstes |
| `--transcribe-language` | `SAMU_TRANSCRIBE_LANGUAGE` | erkannt | die gesprochene Sprache, etwa `de` |

## Konten

Konten werden auf dem Server verwaltet; solange die
[Registrierung](#registrierung) aus ist, legt niemand eins über HTTP an:

```sh
docker compose exec -it server samu-server account add privat
docker compose exec -it server samu-server account list
docker compose exec -it server samu-server account passwd privat   # beendet seine Sitzungen
docker compose exec server samu-server account disable privat      # beendet seine Sitzungen
```

Ein Konto löschen entfernt seine Datenbank, Anhänge und Sitzungen
endgültig. Der laufende Server hält das Konto, darum erst anhalten:

```sh
docker compose stop server
docker compose run --rm server account delete privat
docker compose start server
```

Wer samudo mit etwas darin ausprobieren will, legt ein Konto mit dem
Demo-Inhalt an, deutsch oder englisch: Ordner, Projekte und Aufgaben, Tags
mit zwei Abläufen, Habits mit ihrer Vergangenheit, Perspektiven, Scopes und
Vorlagen, die Daten vom heutigen Tag aus gerechnet. Er kommt nur in ein
neues Konto:

```sh
docker compose exec -it server samu-server account add probe --demo de
```

Mit `--caldav <adresse>` bekommt das Konto auch die Kalender und Termine
der Demo, auf einem CalDAV-Server, dessen Benutzer die Konten sind, wie bei
der öffentlichen Demo; das Passwort kommt aus `SAMU_DEMO_CALDAV_PASSWORD`,
und `SAMU_SECRET_KEY` muss gesetzt sein wie für den Server.

Jedes Konto ist eine eigene Datenbank. Lieber eins fürs Private und eins
für die Arbeit, statt beides zu mischen.

Dann von einem Client aus anmelden: die Web-Oberfläche unter der eigenen
Adresse, die [Kommandozeile](cli.md#einrichten) oder die
[Android-App](android.md) koppeln.

## Registrierung

`SAMU_REGISTRATION` entscheidet, ob Leute selbst ein Konto anlegen, auf der
Anmeldeseite („Konto anlegen“) oder mit `samu register`:

- `off` (Vorgabe): nur `samu-server account add`.
- `demo`: jeder kann ein **Demo-Konto** anlegen. Es bekommt immer den
  Demo-Inhalt und wird `SAMU_DEMO_ACCOUNT_DAYS` Tage (7) nach seiner
  letzten Benutzung mit allem darin gelöscht; das Banner der
  Web-Oberfläche nennt den Tag. Der Knopf „Ausprobieren“ legt eins an,
  ohne etwas zu fragen: der Server wählt Name und Passwort und zeigt sie
  einmal.
- `open`: jeder kann ein gewöhnliches Konto anlegen, auf Wunsch mit dem
  Demo-Inhalt. Gedacht für Familie oder Verein. **Dann liegen fremde
  Daten auf dem eigenen Server**: in den Sicherungen, unter der eigenen
  Verantwortung für den Datenschutz, und löschen können das Konto nur sie
  selbst (oder man selbst mit `account delete`).

Höchstens zehn Konten entstehen pro Stunde, und höchstens
`SAMU_REGISTRATION_MAX` (100) registrierte Konten gibt es gleichzeitig;
Konten aus `account add` zählen nicht. Ein registriertes Konto kann sich
selbst löschen („Einstellungen > Konto“ im Web, `samu account delete`),
eins aus `account add` nicht.

Jedes Konto, registriert oder nicht, kann den Demo-Inhalt nachträglich
holen („Demodaten holen“ unter „Einstellungen > Konto“, `samu demo de`):
ein leeres Konto bekommt alles, eins mit Inhalt nur einen Ordner und einen
Tag „Demo“ mit allem darunter, die wieder gehen, wenn man den Ordner samt
Aufgaben löscht. Kalender gibt es nur mit `account add --demo --caldav`.

## Wie weit es trägt

Ein Prozess `samu-server` bedient alle Konten. Jedes Konto ist eine
eigene SQLite-Datei unter `accounts/`, geöffnet bei seiner ersten Anfrage;
Konten und Sitzungen liegen in `server.db`. Konten warten beim Schreiben
nicht aufeinander, und ein Konto ist eine Datei zum Sichern,
Wiederherstellen oder Löschen.

Gedacht ist das für einen Rechner: einen Haushalt, einen Verein, ein
kleines Team. Was es nicht kann:

- **Eine Instanz je Datenverzeichnis.** Der Server, der ein Konto
  geöffnet hat, hält dessen Datei; ein zweiter Server auf demselben
  Verzeichnis wird bei jedem Konto abgewiesen, das der erste benutzt. Es
  gibt also keine Lastverteilung und kein Umschalten bei Ausfall: Ist der
  Rechner weg, ist samu weg. Dafür gibt es die [Sicherung](#sicherung).
- **Kein Teilen zwischen Konten.** Jedes Konto sieht nur seine eigenen
  Objekte.

## Der geheime Schlüssel

Kalenderquellen (Abo-Adressen, CalDAV-Passwörter) liegen verschlüsselt in
der Datenbank des Kontos. Der Schlüssel ist `SAMU_SECRET_KEY`, 32 Bytes in
Base64. Ohne ihn wird kein Kalender abgerufen. Ihn **getrennt** von den
Datenbanken sichern: Wer beides hat, hat die Quellen; wer den Schlüssel
verliert, setzt jede Quelle neu (`samu calendar source`).

## Transkripte

Eine Sprachnotiz ist eine Aufgabe mit einer Aufnahme. Mit einem
Transkript-Dienst macht der Server aus dem Gesprochenen Titel und Notiz
der Aufgabe und zeigt den Text unter jeder Aufnahme, für die er gefragt
wird. samudo bringt kein Sprachmodell mit: man zeigt es auf einen Dienst.

- **Ohne `SAMU_TRANSCRIBE_URL` verlässt keine Aufnahme den Server.**
  Sprachnotizen behalten dann den Titel „Sprachnotiz *Datum Uhrzeit*“.
  Mit ihr geht jede Aufnahme, die um ein Transkript bittet, an diese
  Adresse; also einen Dienst wählen, dem man anvertraut, was man sagt.
- Der Dienst muss das Format von OpenAIs
  `POST /v1/audio/transcriptions` sprechen: ein Multipart-Formular mit
  `file`, `model` und wahlweise `language`, beantwortet mit JSON, das
  `text` enthält. `SAMU_TRANSCRIBE_URL` ist die ganze Adresse dieses
  Endpunkts.
- Selbst gehostete Dienste, die das tun:
  - [speaches](https://github.com/speaches-ai/speaches) bedient diesen
    Pfad, wie er ist; eines seiner Modelle in `SAMU_TRANSCRIBE_MODEL`
    nennen.
  - Der Server von [whisper.cpp](https://github.com/ggml-org/whisper.cpp),
    gestartet mit `--inference-path /v1/audio/transcriptions --convert`.
    `--convert` braucht `ffmpeg` daneben: die App nimmt AAC in MPEG-4 auf
    und die Web-UI Opus in WebM, und whisper.cpp liest von sich aus
    keines von beiden.
- Der Server schickt eine Aufnahme nach der anderen und wartet bis zu
  15 Minuten auf die Antwort; ein kleiner Rechner reicht also. Ein
  Fehlschlag bleibt mit seinem Grund am Anhang stehen und lässt sich neu
  anfragen. Was beim Stoppen des Servers wartete, geht beim Start hinaus.

## Eine öffentliche Instanz

Ein Server, den jeder erreichen kann, sollte begrenzen, was ein Fremder
damit anstellen kann:

- **Anmeldungen** werden immer gebremst: Nach fünf gescheiterten
  Anmeldungen eines Kontonamens in 15 Minuten wartet dieser Name, bis die
  15 Minuten um sind (HTTP 429); eine gelungene Anmeldung vergisst die
  Fehlschläge. Höchstens zwei Passwörter werden gleichzeitig geprüft.
  Zwanzig falsche Kopplungscodes in 15 Minuten halten das Koppeln genauso
  an. Ein Neustart vergisst all das.
- **Anhänge**: `SAMU_BLOB_MAX_BYTES` und `SAMU_BLOB_QUOTA_BYTES` setzen,
  damit die Platte nicht vollläuft.
- **Kalender**: Mit `SAMU_SECRET_KEY` ruft der Server jede Adresse ab, die
  ein Benutzer ihm als Kalender gibt, auch im eigenen Netz.
  `SAMU_CALENDAR_HOSTS=cal.example.org` begrenzt das auf die genannten
  Hosts; Weiterleitungen auf andere Hosts werden nicht verfolgt.
- **Registrierung**: aus lassen, oder `demo`, dessen Konten von selbst
  gehen. `open` auf einem öffentlichen Server sammelt fremde Daten.

## Sicherung

Alles liegt im Volume: `server.db` und `accounts/<name>.db`, dazu die
Dateien der Anhänge. Bei angehaltenem Server kopieren, oder im laufenden
Betrieb mit dem Backup von SQLite:

```sh
sqlite3 /pfad/zum/volume/accounts/privat.db ".backup '/backup/privat.db'"
```

Vor jedem Update sichern, das eine Datenbank-Migration mitbringt.

## Export und Wiederherstellung

Ein Export ist ein einfaches Verzeichnis, das man mit einem Texteditor,
`jq` oder `git diff` lesen kann, ohne samudo:

```sh
samu export ~/samu-export --org                                  # von einem Client, angemeldet
docker compose exec server samu-server export --account privat /export/privat   # auf dem Server, auch im laufenden Betrieb
```

Es enthält `manifest.json`, eine JSON-Datei je Art von Objekt
(`tasks.json`, `projects.json`, `comments.json`, …), `logbook.json`, die
Dateien der Anhänge unter `files/<sha256>`, eine `README.md`, die alles
beschreibt, und mit `--org` eine Gliederung `samu.org`. Derselbe Stand
ergibt dieselben Bytes, also taugt das Verzeichnis gut für git:
zeitgesteuert exportieren und committen, und man hat einen lesbaren
Verlauf. `deploy/export-git.sh` mit den systemd-Units
`deploy/samu-export.service` und `.timer` ist ein Weg dahin (Pfade und
Konten oben anpassen).

Nicht im Export: gelöschte Objekte, Sitzungen und Passwörter, Hooks und
die Kalenderquellen.

Zum Wiederherstellen den Server anhalten, ein leeres Konto anlegen und den
Export hineinlesen:

```sh
docker compose stop server
docker compose run --rm server account add privat      # nur, wenn es noch nicht existiert
docker compose run --rm server restore --account privat /export/privat
docker compose start server
```

Danach jede Kalenderquelle neu setzen.

## Aktualisieren

Sichern, dann `docker compose pull` und `docker compose up -d` (beim
eigenen Build: das neue Image bauen statt holen). Der Server
migriert die Datenbanken beim Start; eine Datenbank, die eine neuere
Version migriert hat, lässt sich mit einer älteren nicht öffnen, also nicht
ohne die Sicherung zurückgehen. `/health` meldet die laufende Version.

## Webhooks

Der Server nimmt signierte Zustellungen unter `/hooks/<name>` an, etwa von
einem smarten Ring. Siehe [Erfassen vom Index-Ring](ring.md).
