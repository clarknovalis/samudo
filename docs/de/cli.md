# Kommandozeile

`samu` spricht mit dem Server über seine API und deckt sie ganz ab: Jede
Route hat ein Kommando. Andere Clients bauen darauf auf: [Emacs](emacs.md)
und das [Desktop-Widget](desktop-widget.md) rufen `samu` mit
`--format json` auf.

`samu help` und `samu <kommando> --help` listen alles; diese Seite zeigt
die Kommandos nach Aufgabe. IDs lassen sich ganz angeben oder mit ihren
letzten acht Zeichen, wie `samu list` sie ausgibt.

## Einrichten

```sh
make install                               # samu nach ~/.local/bin
vim ~/.config/samu/config.toml
```

```toml
server = "https://samu.example.org"
account = "privat"
device = "laptop"               # der Name dieses Geräts; ohne: der Hostname
web = "https://samu.example.org" # wo samu:-Links öffnen; ohne: der Server

# Was den Wert einer Property auf diesem Gerät öffnet: argv, keine Shell
[open]
jira = ["mein-jira-skript", "{value}"]
```

```sh
samu login                        # fragt nach dem Passwort
samu session                      # wer und wo
samu health                       # antwortet der Server?
```

Mehrere Konten auf einem Rechner sind [Profile](profiles.md) in derselben
Datei; `--profile <name>` oder `SAMU_PROFILE` wählt eins.

### Optionen und Umgebung

| Option | Umgebung | |
|---|---|---|
| `--server` | `SAMU_SERVER` | die Adresse des Servers |
| `--account` | `SAMU_ACCOUNT` | das Konto |
| `--profile` | `SAMU_PROFILE` | ein Profil der Konfigurationsdatei |
| | `SAMU_DEVICE` | der Gerätename |
| | `SAMU_CONFIG` | die Konfigurationsdatei (vorgegeben `~/.config/samu/config.toml`) |
| | `SAMU_STATE` | wo Sitzungs-Tokens liegen (vorgegeben `~/.local/state/samu`) |
| | `SAMU_WEB` | die Web-Oberfläche für `samu:`-Links |
| `--format` | | `plain` (Vorgabe), `json`, `jsonl`, `tsv` |

Daten gehen nach stdout, Meldungen nach stderr.

### Exit-Codes

| Code | Bedeutet |
|---|---|
| 0 | erledigt |
| 1 | etwas anderes ging schief |
| 2 | falscher Aufruf oder ein Profil, das fehlt oder kaputt ist |
| 3 | nicht gefunden (auch: `samu clock out` ohne laufende Uhr) |
| 4 | der Server war nicht erreichbar |
| 6 | nicht angemeldet, oder die Anmeldung schlug fehl |

### Ein Handy koppeln

```sh
samu pair phone          # fragt nach dem Passwort, gibt einen Code und einen QR-Code aus
```

Der Code gilt einmal, für zehn Minuten. In der Android-App scannen oder auf
einem anderen Rechner mit `samu login --code <code>` benutzen.

### Agenten

Ein KI-Assistent oder ein Skript sollte eine eigene Sitzung haben, damit
das, was er schreibt, als das eines Agenten markiert ist:

```sh
SAMU_STATE=~/.local/state/samu-agent SAMU_DEVICE=assistant samu login --agent
```

Eine solche Sitzung lässt sich auf einige Ordner und Projekte mit allem
darunter beschränken; sie sieht dann nichts anderes, nicht einmal die
Inbox. Benannt werden sie mit dem Pfad, wie `samu folder list` ihn zeigt,
ein Projekt auch mit seinem Titel, oder mit der ID. Eine beschränkte
Sitzung meldet sich direkt beim Server an, nicht über die lokale Replik.

```sh
samu login --agent --only arbeit/ki                  # dort lesen und schreiben
samu login --agent --only arbeit/ki --read-only      # nur lesen
samu session                                         # zeigt die Freigabe
```

`samu sessions` listet die Sitzungen des Kontos (nie ihre Tokens), und
`samu session end <id>` beendet eine, aus der Sitzung eines Menschen für
das ganze Konto.

Auf einem Server, der Registrierungen annimmt
([Selbst hosten](self-hosting.md#registrierung)):

```sh
samu register --info                               # was der Server erlaubt
samu register                                      # ein Demo-Konto; Name und Passwort einmal ausgegeben
samu register alice --password-stdin --demo de     # ein eigenes Konto
samu demo de                                       # Demo-Inhalt ins angemeldete Konto
samu account delete                                # ein registriertes Konto, endgültig
```

## Erfassen

```sh
samu add "Fliesenleger anrufen #Bad @telefon d:fr ~15m"
samu add "Milch kaufen" --dry-run         # zeigen, nichts schreiben
samu add --literal "#hashtag im Titel"    # der Text als Titel, wörtlich
samu add "Teilschritt" --parent 4f2a91c3
samu add "Entwurf /woche"                 # mit einer Vorlage
```

Die Zeichen stehen unter [Schnellerfassung](quick-capture.md). Vorlagen:
`samu template list | add | text | remind | key | rename | delete`; die
Erinnerung einer Vorlage ist `planned`, `due:1h`, `defer:30m` oder
`daily@20:00`.

## Nachsehen

```sh
samu list                      # offene Aufgaben mit ihrer Verfügbarkeit
samu list --available --tag telefon
samu list --today              # geplant für heute oder früher
samu list --flagged
samu list --project Bad --all
samu list --search fliesen
samu list --perspective "Kurze Anrufe"
samu list --columns state,due,estimate,worked,prop:jira   # als Tabelle
samu projects                  # Projekte mit ihrer Verfügbarkeit
samu projects --stuck          # aktiv, ohne verfügbare Aufgabe
samu projects --review         # Review fällig
samu forecast                  # überfällig, heute, markiert, nächste Tage, später
samu show 4f2a91c3             # ein Objekt, alle Felder
samu log 4f2a91c3              # sein Logbuch
samu prop 4f2a91c3             # seine Properties, auch geerbte
samu blockers 4f2a91c3         # die Aufgaben, auf die es wartet
```

Der gewählte [Scope](concepts.md#scopes) gilt; `--scope <name>` wählt
einen anderen, `--no-scope` keinen. Ohne Scope fehlt, was einen eigenen
Scope hat; diesen Scope wählen, um es zu sehen.

`--columns` gibt eine Tabelle aus, mit denselben Spaltennamen wie die
Web-Oberfläche (`worked` ist die Zeit auf den Uhren, `remaining` Schätzung
minus Ist; eine Aktionsgruppe zählt ihren Teilbaum); ohne Wert die
Vorgabe-Spalten, eine Tabellen-Perspektive bringt ihre eigenen mit.

Eine Block-Perspektive bekommt vor jedem Block eine Überschrift
(`▸ Titel (n)`), in tsv die Überschrift als erste Spalte, in json die
Blöcke mit ihren Aufgaben.

## Ändern

```sh
samu edit 4f2a91c3 "@besorgung !A d:mo // nach dem Preis fragen"
samu set 4f2a91c3 title "Fliesenleger nochmal anrufen"
samu set 4f2a91c3 flagged true          # Werte sind JSON, sonst ein String
samu set 4f2a91c3 properties.jira ABC-1
samu unset 4f2a91c3 due
samu done 4f2a91c3
samu drop 4f2a91c3 --note "nicht mehr nötig"
samu state 4f2a91c3 Wartet             # ein Zustand seines Ablaufs, per Titel oder Taste
samu state 4f2a91c3 --next
samu block 4f2a91c3 9e01d2aa            # warten, bis 9e01d2aa erledigt ist
samu unblock 4f2a91c3 9e01d2aa
samu remind 4f2a91c3 --before 1d        # einen Tag vor fällig
samu remind 4f2a91c3 --at 2026-11-20T08:30
samu remind 4f2a91c3 --daily 20:00      # jeden Tag; eine Gewohnheit nur, solange offen
samu reminders
samu delete 4f2a91c3 --dry-run          # zählen, was ginge
```

`samu delete` auf einem Projekt verschiebt seine Aufgaben in die Inbox;
`--tasks delete` löscht sie mit, `--tasks <projekt>` verschiebt sie dorthin.

### Viele auf einmal

```sh
samu list --tag telefon --format tsv | samu bulk done -
samu bulk move --to Bad 4f2a91c3 9e01d2aa
samu bulk date due morgen -            # IDs von stdin
```

`samu bulk` nimmt `done`, `drop`, `reopen`, `flag`, `unflag`, `move`,
`tag`, `state`, `date` und `delete`.

### Orte und Tags

```sh
samu folder add Zuhause
samu project add Bad --folder Zuhause
samu tag add Ort/Büro                  # Büro unter Ort
samu tag add Status --exclusive --default-workflow  # ein Ablauf; seine Kinder sind Zustände
samu tag add Status/Wartet --key w     # --ask-note verlangt beim Betreten eine Notiz
samu tag list
samu review Bad                        # als geprüft markieren
```

`rename`, `move` und `delete` wirken auf Ordner, Projekte und Tags, wie
`--help` zeigt.

## Kommentare und Dateien

```sh
samu comment 4f2a91c3 "Angerufen, meldet sich Montag"
echo "längerer Text" | samu comment 4f2a91c3 -
samu comments 4f2a91c3
samu attach 4f2a91c3 foto.jpg
samu attachments 4f2a91c3
samu blob get <sha256> > datei
```

Kommentare sind Org oder Markdown; `samu render` zeigt, wie ein Text
gelesen wird.

## Zeit

```sh
samu clock in 4f2a91c3
samu clock status
samu clock out
samu clock add 4f2a91c3 "gestern 2h"
samu clock list 4f2a91c3
samu report --days 7 --by project
samu report --q "#Bad" --part time
```

Der Bericht zählt die eigene Arbeit; `--q "autor:agent"` zeigt die der
Agenten, `autor:alle` die aller (englisch `author:agent`, `author:all`
geht ebenso).

## Kalender

```sh
samu calendar discover https://dav.example.org --username alice --password-stdin --add
samu calendar add Feiertage --url https://example.org/feiertage.ics
samu calendar discover --from Privat --add Arbeit      # weitere Kalender auf demselben Server
samu calendar import Privat export.ics
samu calendar list
samu calendar refresh <id>
samu appointments --from 2026-11-16 --to 2026-11-22
samu appointment add Zahnarzt --start 2026-11-20T10:00 --end 2026-11-20T11:00 --calendar Privat
```

`--from` übernimmt Nutzer und Passwort von einem schon vorhandenen
CalDAV-Kalender; sie bleiben auf dem Server. `samu calendar import`
schickt jeden Termin der Datei unverändert an den Server des Kalenders,
ersetzt einen mit derselben UID und meldet auf stderr, was ausgelassen
wurde oder scheiterte (Exit 1, wenn etwas scheiterte). Kalender abzurufen
braucht einen geheimen Schlüssel auf dem Server (siehe
[Selbst hosten](self-hosting.md#der-geheime-schlüssel)).

```sh
samu alarms                                  # Erinnerungen der nächsten 24 Stunden
samu alarms --to 2026-11-22 --no-tasks       # nur Termine, bis zu diesem Tag
```

## Scopes, Perspektiven, Einstellungen

```sh
samu scope add --only Arbeit Arbeit    # nur, was der Ordner Arbeit abdeckt; --except für den Rest
samu scope use Arbeit                  # auf diesem Gerät
samu scope use auto                    # den Zeitplänen der Scopes folgen
samu scope use --none
samu scope schedule Arbeit "mo-fr 08:00-17:00"
samu set <id> own_scope Arbeit         # ein Ordner, Projekt oder Tag nur in Arbeit
samu unset <id> own_scope              # wieder überall
samu perspective add "Kurze Anrufe" '@telefon ~<15m'
samu perspective view "Kurze Anrufe" table --columns state,due,estimate,worked
samu perspective add Morgen --blocks :today "Kurze Anrufe=Am Schreibtisch" :flagged
samu perspective blocks Morgen                   # zeigt die Blöcke; --clear macht sie einfach
samu perspective dedupe Morgen off               # eine Aufgabe in jedem Block, der sie zeigt
samu settings                          # die Einstellungen des Kontos
samu settings set remind-time 08:00
samu settings set due-soon P3D
```

## Gewohnheiten

`samu habit` legt [Gewohnheiten](habits.md) an, trägt sie ein und zeigt sie.

## Live und roh

```sh
samu events                    # eine Zeile je Änderung, wenn sie geschieht
samu changes --after 0         # was sich nach einer Änderungsnummer geändert hat
samu apply changes.json        # rohe Änderungen senden; siehe API
```

## Import und Export

```sh
samu import org ~/org --dry-run        # die offenen Aufgaben eines Org-Agenda-Verzeichnisses
samu import org ~/org
samu export ~/samu-export --org        # das Konto als JSON-Dateien, und samu.org
samu export-org ~/samu-export          # samu.org neu, nur aus dem Verzeichnis
```

Der Import kann wieder laufen: Was ein früherer Lauf hereingebracht hat,
bleibt, wie es ist. Der Export ist unter
[Selbst hosten](self-hosting.md#export-und-wiederherstellung) beschrieben.

## Hooks

`samu hook add | list | secret | remove | send` verwalten Webhooks, an die
ein Absender von außen liefert; siehe [Erfassen vom Index-Ring](ring.md).

## Links öffnen

`samu open <id> <schlüssel>` öffnet den Wert einer Property mit dem
Kommando dieses Geräts aus `[open]`, sonst mit `xdg-open`.
`samu open-uri` nimmt `samu:<id>`- und `samu-open:`-Adressen aus dem
Browser; `make install` meldet es an.

## zsh-Vervollständigung

```sh
source <(COMPLETE=zsh samu)      # in ~/.zshrc, nach compinit
```

Sie ergänzt Kommandos, Optionen, Aufgaben-IDs mit Titeln, Zustände und in
`samu add` die `@tags`, `#projekte`, Daten, Schätzungen und
Wiederholungen, live vom Server.
