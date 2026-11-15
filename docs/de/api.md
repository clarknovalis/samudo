# HTTP-API

Jeder Client von samudo, die Web-Oberfläche eingeschlossen, benutzt diese
API und nichts sonst. Die [Kommandozeile](cli.md) deckt sie ganz ab, also
ist `samu <kommando> --format json` der schnellste Weg, eine Antwort zu
sehen. Die Wire-Typen sind das Rust-Crate `samu-api` (nur serde, eigens
versioniert); JSON-Beispiele jedes Typs liegen in
[`crates/samu-api/tests/fixtures/`](../crates/samu-api/tests/fixtures/).

## Konventionen

- IDs sind UUIDv7, Zeitstempel ISO 8601 in UTC mit `Z`, Zeitspannen
  ISO-8601-Dauern (`PT15M`, `P2D`).
- Ein Datum ist `{"date": "2026-11-20"}` (ein ganzer Tag, keine Zone) oder
  `{"at": "2026-11-20T09:00:00+01:00[Europe/Berlin]"}` (ein Zeitpunkt mit
  seiner Zone).
- Felder, die ein Leser nicht kennt, bleiben erhalten; Enum-Werte einer
  neueren Version lesen sich als `unknown` und werden nie zurückgeschrieben.
- Fehler sind ein Statuscode mit
  `{"error": "not_found", "message": "…"}`; Arten sind etwa
  `unauthorized`, `not_found`, `offline` (eine Replik ohne ihren Server).

## Anmelden

```http
POST /login
{"account": "privat", "password": "…", "device": "laptop"}
```

antwortet mit einem Sitzungs-`token` und seinem Ablauf und setzt für
Browser ein Cookie. Das Token als `Authorization: Bearer <token>` senden.
Mit `"kind": "agent"` ist die Sitzung die eines Agenten: Was sie schreibt,
ist als solches markiert. `GET /session` zeigt die Sitzung sowie Zone und
Einstellungen des Kontos; `POST /logout` beendet sie.

Koppeln: `POST /pair/code` (noch einmal mit dem Passwort) macht einen
Einmal-Code für zehn Minuten; `POST /pair` tauscht ihn auf dem neuen Gerät
gegen eine Sitzung.

## Lesen

| Route | |
|---|---|
| `GET /available` | offene Aufgaben mit Verfügbarkeit, jedem Grund, Zustand und nächstem Zustand; Filter: nur verfügbar, heute, Projekt, alle, Scope, Perspektive |
| `GET /available/projects`, `/available/folders` | Projekte (stuck, Review fällig) und Ordner |
| `GET /forecast` | die Abschnitte des Forecasts, jede Zeile mit dem Grund, warum sie dort steht |
| `GET /objects/{id}` | ein Objekt beliebiger Art |
| `GET /objects/{id}/logbook` | sein Verlauf |
| `GET /changes?after=<seq>` | jedes Objekt, das sich nach einer Änderungsnummer geändert hat, für den Cache eines Clients |
| `GET /habits`, `/habits/{id}` | Stand, Serien und Einträge der Gewohnheiten |
| `GET /appointments` | Termine einiger Tage, Wiederholungen aufgefaltet |
| `GET /alarms` | die Erinnerungen an Aufgaben und Termine, die in einem Fenster von höchstens sieben Tagen fällig werden, nach Zeit |
| `GET /clocks/report`, `GET /report` | Zeit und Bericht |
| `GET /scopes/scheduled` | welchen Scope die Zeitpläne jetzt wählen, und bis wann |
| `GET /blobs/{sha256}` | die Datei hinter einem Anhang |
| `GET /export` | das ganze Konto (siehe [Selbst hosten](self-hosting.md#export-und-wiederherstellung)) |
| `GET /health` | `{"status": "ok", "version": …, "api": …}`, ohne Sitzung |

## Schreiben

Fast jedes Schreiben ist eine Liste von **Änderungen** an `POST /changes`.
Eine Änderung ist eine von:

```json
{"op": "create", "object": "<id>", "kind": "task", "value": {…ganzes Objekt…}, "hlc": …}
{"op": "set",    "object": "<id>", "field": "due", "value": {"date": "2026-11-20"}, "hlc": …}
{"op": "add",    "object": "<id>", "field": "tags", "element": "<tag id>", "position": 0, "hlc": …}
{"op": "remove", "object": "<id>", "field": "tags", "element": "<tag id>", "hlc": …}
{"op": "delete", "object": "<id>", "hlc": …}
```

`set` mit `null` leert ein Feld; eine Property ist das Feld
`properties.<schlüssel>`. `create` ist über die ID idempotent, also macht
ein Client die ID selbst und darf dieselbe Änderung zweimal senden.
Änderungen an Zuständen und am Erledigen dürfen eine `note` fürs Logbuch
tragen.

`hlc` ist ein Stempel einer hybriden logischen Uhr, `{"wall": <ms>,
"counter": n, "device": "laptop"}`. Der Server entscheidet **je Feld**: Der
spätere Stempel gewinnt. Jede Änderung bekommt ein Ergebnis:

```json
{"outcome": "applied", "seq": 185}
{"outcome": "superseded", "by": {"wall": …, "counter": 0, "device": "phone"}}
{"outcome": "rejected", "reason": "field \"due\" expects {\"date\"} or {\"at\"}"}
```

Schreiben mit eigener Logik hat eigene Routen:

| Route | |
|---|---|
| `POST /capture` | eine Zeile der [Schnellerfassung](quick-capture.md): Vorschau, anlegen oder auf eine Aufgabe anwenden |
| `POST /objects/{id}/delete` | löschen, mit dem, was daran hängt |
| `PUT /blobs/{sha256}` | eine Datei hochladen; danach einen Anhang anlegen, der sie nennt |
| `POST /render` | eine Notiz oder ein Kommentar als Dokumentbaum |
| `PUT /calendars/{id}/source`, `POST /calendars/{id}/refresh`, `POST /calendars/discover` | Kalenderquellen; eine Quelle kann Nutzer und Passwort von einem anderen Kalender auf demselben Host übernehmen (`credentials_from`) |
| `POST /calendars/{id}/discover` | die übrigen Kalender auf dem Server eines CalDAV-Kalenders, mit dessen gespeichertem Nutzer und Passwort |
| `POST /calendars/{id}/import` | eine iCalendar-Datei (`text/calendar`) in einen schreibbaren CalDAV-Kalender |
| `POST /appointments/{id}/edit` | einen Termin oder Vorkommen davon ändern |
| `GET`, `POST /hooks` | Webhooks des Kontos |
| `POST /hooks/{name}` | eine signierte Zustellung von außen, ohne Sitzung |

## Live

`GET /events` ist ein Strom von Server-Sent Events. Ein Ereignis
`changed` trägt nur IDs, nie Inhalt:

```json
{"seq": 184, "gone": ["…"], "added": ["…"], "changed": ["…"], "config": false}
```

Ein Client lädt neu, was er zeigt; `config: true` heißt, die Einstellungen
des Kontos haben sich geändert. Ein Ereignis `reset` (`{"seq": 190}`)
heißt: alles, was man zeigt, von Grund auf neu laden.

## Routen der Web-Oberfläche

`/` und `/lib/…` liefern die Web-Oberfläche aus dem Binary; `/o/{id}`
öffnet darin ein Objekt.
