# Architektur

Für alle, die mitbauen: wie die Teile zusammenpassen, und warum. Zu jeder
Entscheidung gibt es einen Decision Record unter `docs/adr/` (englisch);
ein Kommentar mit `ADR NNNN` verweist darauf. Die Records vor 0133 werden
noch ins Repository übersetzt; bis dahin steht, was für eine Änderung
zählt, hier, im Code oder in der Commit-Message.

## Grundsätze

- **Der Server hält die Wahrheit.** Clients halten einen Lese-Cache und
  einen Ausgang von Feldänderungen, gestempelt mit einer hybriden logischen
  Uhr (HLC) und ihrem Gerät. Der Server entscheidet Last Writer Wins je
  Feld. Es gibt kein zweites Op-Log.
- **Ein Konto, eine Datenbank.** Jedes Konto ist eine eigene
  SQLite-Datei; `server.db` hält Konten und Sitzungen.
- **Verfügbarkeit wird berechnet, nie gespeichert**, an genau einer
  Stelle: `samu-core`. Server und Handy rufen denselben Code auf.
- **Zeit:** Ein ganzer Tag ist ein lokales Datum ohne Zone; eine Uhrzeit
  ist ein Zeitpunkt plus Zone. Wiederholung ist eine RRULE plus einer von
  drei Modi.
- **Die API ist der Kern.** Jede Oberfläche ist ein Client; jede Route hat
  ein Kommando in der CLI.

## Grundentscheidungen

**Sync.** Jedes Konto hat eine Änderungsnummer, `seq`, und jedes Feld den
Stempel seiner letzten Änderung: eine hybride logische Uhr
`{wall, counter, device}`. Eine Änderung ist eine von fünf Operationen,
`create`, `set`, `add`, `remove` und `delete`, und der Server beantwortet
jede mit `applied`, `superseded` (eine neuere Änderung gilt schon, und der
Client erfährt es, statt zu raten) oder `rejected`. Clients holen nach
Ankunft, nicht nach Uhr: `GET /changes?after=<seq>` liefert alles danach,
oder `reset`, wenn der Client zu lange weg war. Ein gelöschtes Objekt
bleibt als Grabstein, damit ein Client, der offline war, es mitbekommt.
Neue Objekte bekommen ihre UUIDv7 auf dem Client, ein wiederholtes
`create` schadet nicht. Online-Clients (Web, CLI, Emacs) schreiben direkt,
der Server stempelt für sie; nur App und Replik haben einen Ausgang. Last
Writer Wins ist eine reine Funktion in `samu-core` (`clock::wins`).

**Unbekannte Werte.** Jedes Enum auf dem Draht hat den Rückfallwert
`unknown`. Er wird gelesen, nie geschrieben: Ein Client, der ein Objekt mit
einem Wert zeigt, den er nicht kennt, lässt dieses Feld in Ruhe, damit ein
alter Client nie überschreibt, was ein neuerer gesetzt hat. Ein neuer
Enum-Wert ist darum eine Nebenversion von `samu-api` wie ein neues Feld;
einen entfernen oder umdeuten ist eine Hauptversion.

**Speicher.** Eine Tabelle `object` für alle Arten trägt die ID, die `seq`
der letzten Änderung und den Grabstein; jede Art hat eine Tabelle mit
getypten Spalten, und `stamp` hält den Stempel jedes Felds. Eine
Datenbankdatei schreibt immer nur ein Prozess (Dateisperre).

**Autor.** Den Autor einer Änderung, `{device, kind}`, setzt der Server aus
der Sitzung, die sie bringt, auch bei Änderungen aus einem Ausgang; was ein
Client behauptet, zählt nicht. `kind` ist `human` oder `agent` und wird bei
der Anmeldung gewählt (`samu login --agent`), damit Logbook und Bericht die
Arbeit eines Menschen von der eines Agenten trennen.

**Kalender.** Adresse und Zugangsdaten eines Kalenders sind ein Geheimnis:
versiegelt mit XChaCha20-Poly1305 unter `SAMU_SECRET_KEY`, nie in
`/changes`, nie in einer Antwort. Der Server holt die Kalender; Termine
eines schreibbaren CalDAV-Kalenders lassen sich über `/changes` ändern wie
jedes Objekt, und der Server schreibt sie zum Kalenderserver zurück.

**Ausstieg.** `samu export` schreibt ein Verzeichnis, je Art eine
JSON-Datei in der Wire-Form von `samu-api`, sortiert und stabil für
`diff`, samt Anhängen. Es ist Sicherung und ein Ausweg, der kein samudo
zum Lesen braucht.

**Clients.** Die Web-Oberfläche ist reines JavaScript in Modulen ohne
Build-Schritt, eingebettet ins Server-Binary. Die Android-App ist Kotlin
und Compose über `samu-mobile` (uniffi). Auf einem Rechner beantwortet die
Replik dieselbe API auf dem Loopback aus einem Spiegel, auch während der
Server weg ist. Die Texte der Oberfläche sind deutsch und dienen als
Schlüssel; eine englische Tabelle übersetzt sie.

## Crates

```
crates/
  samu-api/      Wire-Typen (nur serde), eigens versioniert und veröffentlicht
  samu-core/     Modellregeln, Verfügbarkeit, Wiederholung, Schnellerfassung,
                 Suche, Forecast, Erinnerungen, Bericht, Gewohnheiten; kein I/O, kein async
  samu-store/    SQLite (rusqlite), nummerierte Migrationen
  samu-server/   axum: API, Server-Sent Events, Hooks, Kalender, die Web-Oberfläche
                 eingebettet; Binary `samu-server` (serve, replica, profile, front)
  samu-client/   HTTP-Client (rustls) auf samu-api; geteilt von CLI und App
  samu-cli/      Binary `samu`
  samu-replica/  Spiegel, Ausgang und Sync eines Clients, ohne uniffi;
                 geteilt von App und lokaler Replik
  samu-mobile/   Rust-Seite der Android-App (uniffi-Bindings)
```

Abhängigkeiten zeigen in eine Richtung: `samu-api` ← `samu-core` ←
`samu-store` ← `samu-server`; `samu-api` ← `samu-client` ← `samu-cli`,
`samu-mobile`; `samu-core`, `samu-client` ← `samu-replica` ←
`samu-mobile`, `samu-server`; `samu-core` ← `samu-cli`, die damit die
Suchsprache liest. Nichts hängt von `samu-server` ab;
`samu-api` hängt von nichts im Workspace ab.

## Weitere Teile

| Verzeichnis | |
|---|---|
| `web/` | die Web-Oberfläche: ES-Module, kein Framework, kein Build-Schritt; eingebettet ins Server-Binary (`crates/samu-server/src/web.rs` listet die Dateien) |
| `android/` | Kotlin und Compose über `samu-mobile` |
| `emacs/` | `samu.el`, über die CLI |
| `deploy/` | systemd-Units, Export-Skript |
| `docs/` | diese Dokumentation; `docs/de/` ist die deutsche Übersetzung |
| `site/` | die Doku-Site (Hugo), gebaut aus `docs/` |
| `testdata/` | nur synthetische Daten |

## Datenfluss

```
 Web ────┐
 CLI ────┼── HTTPS ──► samu-server ──► accounts/<name>.db
 Emacs ──┤  (über CLI)     │
 Widget ─┘                 └─ GET /events (SSE) ──► jeder Client
 Android-App: lokaler Spiegel + Ausgang ──► POST /changes, GET /changes
 Replik auf einem Rechner: dieselbe API auf 127.0.0.1, Spiegel + Ausgang
```

## Bauen und prüfen

Stabiles Rust (Edition 2024), siehe `rust-toolchain.toml`;
`make check-public` braucht dazu Node.js (ohne Pakete) für die Tests des
Webs.

```sh
make check-public # fmt, clippy -D warnings, Tests, Paket samu-api, Web, samu.el
make install      # samu nach ~/.local/bin
cargo build -p samu-server
target/debug/samu-server --data /tmp/samu serve --listen 127.0.0.1:8492
```

Regeln, die das Projekt einhält:

- Jede neue Funktion bekommt Tests, jeder Bugfix einen Regressionstest.
  Tests nutzen nur synthetische Daten und rufen keine Dienste von außen
  auf.
- Jede API-Route bekommt im selben Commit ein CLI-Kommando. Ausgabe der
  CLI: Daten auf stdout, Meldungen auf stderr,
  `--format plain|json|jsonl|tsv`, festgelegte Exit-Codes.
- Jede Änderung an `samu-api` kommt mit einer JSON-Fixture unter
  `crates/samu-api/tests/fixtures/`.
- Schema-Änderungen nur als nummerierte Migrationen in `samu-store`.
- Kein `unsafe` außerhalb von FFI, kein `unwrap()` außerhalb von Tests.

## Schwesterprojekt

samudo hat ein Schwesterprojekt, *catastr*, einen Speicher für Dokumente
und Wissen. Aufgaben verlinken dessen Objekte per URI (`catastr:<id>`), und
catastr liest samudo als Quelle über dieselbe API und das Crate
`samu-api`. Keins fasst die Datenbank des anderen an.
