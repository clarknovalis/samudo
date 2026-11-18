# Konzepte

samudo leiht sich aus mehreren Todo-Welten: Projekte, Verfügbarkeit und
Perspektiven von OmniFocus, Erfassen, Refile und Zeiterfassung von
org-mode, Abläufe mit Zuständen von Jira. Diese Seite erklärt die Teile
einmal; die Seiten der einzelnen Oberflächen sagen nur, wo man sie dort
findet. Die Gründe hinter dem Aufbau stehen unter
[Architektur](architecture.md).

## Ein Server, viele Clients

- **Der Server ist der einzige Schreiber.** Jede Aufgabe liegt in einer
  Datenbank auf dem Server. Kommandozeile, Web-Oberfläche, Android-App,
  Emacs und Desktop-Widget sind Clients seiner HTTP-API.
- **Offline ist eine Warteschlange, keine zweite Wahrheit.** Die
  Android-App und die optionale lokale Replik auf einem Rechner halten eine
  Kopie und einen *Ausgang* mit Änderungen. Erreichen sie den Server
  wieder, entscheidet er je Feld: Die spätere Änderung gewinnt (Last Writer
  Wins, gestempelt mit einer hybriden logischen Uhr).
- **Ein Konto, eine Datenbank.** Privat und dienstlich sind zwei Konten
  mit zwei getrennten SQLite-Dateien, nicht ein Feld an jeder Aufgabe. Ein
  Rechner kann über [Profile](profiles.md) bei mehreren Konten zugleich
  angemeldet sein.
- **Verfügbarkeit wird berechnet, nie gespeichert.** Ob eine Aufgabe jetzt
  erledigt werden kann, rechnet der Server aus (und, mit demselben Code,
  die App, solange sie offline ist). Kein Client entscheidet das selbst.

## Orte: Inbox, Ordner, Projekte, Aktionsgruppen

| Was | Was es ist |
|---|---|
| **Inbox** | Aufgaben ohne Projekt. Alles, was ohne `#projekt` erfasst wird, landet hier. |
| **Ordner** | Ein Baum aus Ordnern hält die Projekte. Ein Ordner ist aktiv oder verworfen; verwerfen blendet alles darunter aus. |
| **Projekt** | Ein Ziel mit Aufgaben. Seine *Art* ist **parallel** (jede offene Aufgabe ist verfügbar), **sequenziell** (nur die erste offene ist es) oder **Einzelaktionen** (eine Liste loser Aufgaben, die nie „stuck“ ist). Sein *Status* ist aktiv, ruht, erledigt oder verworfen. |
| **Aktionsgruppe** | Eine Aufgabe mit Kindern. Sie ist selbst nicht verfügbar, solange sie offene Kinder hat; sie kann sequenziell sein und sich mit ihrem letzten Kind selbst erledigen. |

Ordner, Projekte und Tags können ein Emoji als Symbol tragen. Ein Ordner,
ein Projekt oder ein Tag kann einen **eigenen Scope** nennen („Nur im
Scope: Work“): Dann erscheint es nur in diesem [Scope](#scopes), ob der
Scope *nur* zeigt, was er ankreuzt, oder *alles außer* dem, und sonst
nirgends, auch nicht ohne Scope. Das ist praktisch für einen
Arbeitsbereich, den man zu Hause nicht sehen will, oder für Gewohnheiten.
In seinem eigenen Scope braucht es kein Kreuz; angekreuzt in einem Scope
„alles außer“ fällt es heraus wie alles andere. In einem Ordner mit
eigenem Scope entscheidet der nächste Ordner, der einen hat.

Damit nichts scheinbar verschwindet, sagt samudo, wohin: Die Suche nennt,
wie viele Treffer nur ein anderer eigener Scope zeigt, mit einem Klick
dorthin (`samu list --search` meldet es auf stderr); die Projektauswahl
kennzeichnet solche Projekte mit „nur im Scope Work“; und wer eine Aufgabe
dorthin verschiebt, bekommt eine Meldung mit „Dorthin“ und „Rückgängig“.

## Aufgaben

Eine Aufgabe hat einen Titel, eine Notiz (Org oder Markdown), einen Ort
und wahlweise:

- **Daten**, jedes entweder ein ganzer Tag (ein Datum ohne Zone) oder ein
  Zeitpunkt (eine Uhrzeit mit der Zone des Kontos):
  - **Start**: vor diesem Datum nicht verfügbar;
  - **Fällig**: muss bis dahin erledigt sein; eine einstellbare Zeit vorher
    ist sie *bald fällig* (vorgegeben zwei Tage, je Aufgabe änderbar),
    danach *überfällig*;
  - **Geplant**: wann man sie vorhat. Mit Uhrzeit und Dauer ist sie ein
    Block in der Kalenderansicht.
- **Markiert**, **Priorität** A, B oder C (keine sortiert hinter C) und
  eine **Schätzung** (`~30m`).
- **Wiederholung**: eine RRULE (RFC 5545) mit einem von drei Modi:
  - *fest* (`every:`): Die nächsten Daten folgen der Regel, egal wie spät
    man erledigt hat; mit Aufholen (`every+:`) werden verpasste Termine bis
    heute übersprungen;
  - *fällig nach Erledigen* (`every!:`): Das nächste Fälligkeitsdatum zählt
    ab dem Tag, an dem man erledigt hat;
  - *Start nach Erledigen*: Das nächste Startdatum zählt ab dem Erledigen.
  Erledigen einer wiederholten Aufgabe legt die nächste als neue Aufgabe an.
- **Tags** in einem Baum, mit einer Reihenfolge an jeder Aufgabe. Ein Tag
  ist aktiv, ruht oder ist verworfen; eine Aufgabe mit einem nicht aktiven
  Tag ist nicht verfügbar.
- **Properties**: freie Schlüssel und Werte, geerbt von Aktionsgruppe und
  Projekt. Ein Schlüssel kann eine Link-Vorlage haben, sodass `jira=ABC-1`
  den eigenen Tracker öffnet.
- **Links** zu anderen Objekten: `samu:<id>`, `https:`, `mid:` (eine Mail),
  `catastr:` (das Schwesterprojekt) und andere. Ein Link darf auf etwas
  zeigen, das sich nicht auflöst.
- **Blocker**: „blockiert durch“ eine andere Aufgabe. Die Aufgabe wartet,
  bis jene erledigt ist; samudo lehnt eine Aufgabe ab, die sich selbst
  blockiert oder einen Kreis bildet.

### Verfügbarkeit und ihre Gründe

Jede Ansicht zeigt Aufgaben mit ihrer Verfügbarkeit und **jedem Grund**,
der zutrifft, damit man sieht, *warum* etwas nicht verfügbar ist: erledigt,
verworfen, zurückgestellt bis zu einem Datum, Projekt nicht aktiv, Ordner
verworfen, Tag nicht aktiv, wartet in einer Folge, hat offene Kinder, die
Aktionsgruppe ist nicht verfügbar, blockiert durch eine andere Aufgabe.

Die Ansichten bieten drei Filter: **Verfügbar**, **Verbleibend** (alles
Offene, auch was wartet) und **Alle** (auch Erledigtes und Verworfenes).

## Abläufe und Zustände

Ein Tag kann *exklusiv* sein: Dann ist er ein **Ablauf**, und seine Kinder
sind die **Zustände**, zum Beispiel `Status` mit `Next`, `In Arbeit`,
`Wartet`, `Erledigt`. Eine Aufgabe ist in höchstens einem Zustand eines
Ablaufs. Jeder Zustand kann eine Taste haben (für die Taste `t` im Web),
beim Betreten eine Notiz verlangen, und die Board-Ansicht nimmt die
Zustände als Spalten. Ein Projekt nennt den Ablauf seiner Aufgaben; sonst
gilt der des Kontos.

## Ansichten

| Ansicht | Zeigt |
|---|---|
| **Inbox** | Aufgaben ohne Projekt |
| **Alle Projekte** | den Baum aus Ordnern und Projekten mit ihren Aufgaben |
| **Heute** | geplant für heute oder früher |
| **Forecast** | überfällig, heute, markiert, die nächsten Tage und später, jede Zeile mit dem Grund, warum sie dort steht; die Termine der Kalender neben den Aufgaben |
| **Kalender** | Tag, Woche, Monat oder Liste mit Terminen und geplanten Aufgaben |
| **Markiert** | markierte Aufgaben |
| **Stuck** | aktive Projekte ohne verfügbare Aufgabe (Einzelaktionslisten zählen nie) |
| **Review** | Projekte, deren Review fällig ist, die ältesten zuerst |
| **Bericht** | was erledigt und bearbeitet wurde, und die erfasste Zeit, über Tage |
| **Perspektiven** | die gespeicherten Suchen, siehe unten |

**Gewohnheiten** sind Aufgaben mit einem Ziel, das man Tag für Tag
einträgt; sie stehen in den Listen wie andere Aufgaben, mit einem Knopf
statt des Kreises. Siehe [Gewohnheiten](habits.md).

### Review

Jedes Projekt kann ein Review-Intervall haben. Die Review-Ansicht listet
die Projekte, deren nächstes Review gekommen ist; „geprüft“ setzt das
nächste Review ab heute.

### Perspektiven

Eine Perspektive ist eine gespeicherte Suche: Wörter und Chips der
[Suchsprache](search.md), ein Filter (verfügbar, verbleibend, alle), eine
Gruppierung und Sortierung, und Liste oder Board. Perspektiven stehen in
der Seitenleiste; die Kommandozeile zeigt eine mit
`samu list --perspective <name>`.

Eine **Block-Perspektive** zeigt mehrere Perspektiven und die Ansichten
Inbox, Heute und Markiert untereinander, wie die Block-Agenda von
org-mode: etwa zuerst den Tag, dann das Kurze im Büro, dann das, was auf
andere wartet. Jeder Block behält seine Suche, seinen Filter und sein
Anordnen und kann eine eigene Überschrift tragen. Eine Aufgabe, die
mehrere Blöcke zeigen, steht nur im ersten, solange „Doppelte nur
einmal“ an ist.

## Scopes

Ein **Scope** ist eine Linse über alle Ansichten: Er nennt Ordner, Projekte
und Tags (jeweils mit allem darunter) und sagt, ob die Ansichten **nur**
zeigen, was diese abdecken, oder **alles außer** dem. Dazu kann er haben:

- einen Filter in der Suchsprache, den jede Aufgabe zusätzlich erfüllen
  muss;
- ob die Inbox dazugehört;
- was eine Aufgabe bekommt, die erfasst wird, während der Scope gewählt ist
  (ein Projekt, Tags);
- einen Zeitplan (`mo-fr 08:00-17:00`), sodass Geräte auf „auto“ den Scope
  von selbst wechseln;
- welche Kalender er zeigt, eine Farbe und ein Layout für die Details einer
  Aufgabe.

Der gewählte Scope gehört zum **Gerät**, nicht zum Konto: Der Laptop kann
in „Work“ sein, während das Handy alles zeigt.

## Erfassen

Die meisten Aufgaben entstehen per **Schnellerfassung**: eine Zeile mit
Zeichen für Projekt, Tags, Daten und mehr, zum Beispiel
`Fliesenleger anrufen #Bad @telefon d:fr ~15m`. Siehe
[Schnellerfassung](quick-capture.md). **Vorlagen** speichern eine Zeile mit
Zeichen unter einem Namen (`/habit`) und wahlweise Erinnerungen; eine
Vorlage kann eine Taste zum schnellen Erfassen haben.

Andere Wege hinein: Text aus jeder Android-App an samudo teilen, eine
Sprachnotiz in der Web-Oberfläche, ein Webhook (etwa [von einem smarten
Ring](ring.md)) und der [Org-Import](cli.md#import-und-export).

## Zeit: Uhren und der Bericht

Eine **Uhr** erfasst die Zeit, die man an einer Aufgabe oder einem Projekt
verbringt. Je Konto läuft höchstens eine Uhr; eine andere zu starten hält
die laufende an, eine Aufgabe zu erledigen hält ihre Uhr an. Zeit lässt
sich nachträglich buchen (`z:gestern 2h` in der Schnellerfassung). Der
**Bericht** zeigt über einen Zeitraum, was erledigt wurde, woran gearbeitet
wurde und die erfasste Zeit, gruppiert nach Aufgabe, Projekt, Ordner, Tag
oder Datum.

## Kalender und Termine

samudo liest Kalender von anderswo (ein ICS-Abo oder CalDAV) und zeigt
ihre **Termine** in Forecast und Kalender. Ein CalDAV-Kalender kann als
beschreibbar markiert werden; dann kann samudo dort auch Termine anlegen,
ändern und löschen. Adressen und Passwörter der Kalenderquellen liegen
verschlüsselt auf dem Server.

## Erinnerungen

Eine offene Aufgabe oder ein Projekt mit Fälligkeitsdatum erinnert an
diesem Datum von selbst. Weitere Erinnerungen lassen sich setzen: zu einem
Zeitpunkt oder eine Zeitspanne vor dem Fälligkeits-, Plan- oder Startdatum.
Ein Datum ohne Uhrzeit erinnert zur Erinnerungszeit des Kontos (vorgegeben
09:00). Eine **tägliche** Erinnerung meldet sich jeden Tag zu ihrer Zeit,
solange die Aufgabe offen ist; bei einer Gewohnheit nur an Tagen, an denen
noch etwas fehlt (siehe [Gewohnheiten](habits.md)). Die Android-App zeigt
Erinnerungen als Benachrichtigungen.

## Kommentare, Anhänge und Autoren

Aufgaben und Projekte haben **Kommentare** (Org oder Markdown, gerendert)
und **Anhänge** (jede Datei; Fotos und Sprachaufnahmen direkt aus
Web-Oberfläche oder App). Jedes Objekt hält fest, **wer es angelegt hat**:
das Gerät und ob es ein Mensch oder ein **Agent** war. Agenten (etwa ein
KI-Assistent an der Kommandozeile) melden sich mit einer eigenen Sitzung
an, damit ihre Kommentare und Änderungen als solche erkennbar sind.

## Verlauf und Löschen

Jedes Objekt hat ein **Logbuch** darüber, was ihm geschehen ist:
Zustandswechsel, Erledigen, Verwerfen, Verschieben und Löschen, jeweils mit
dem Gerät und ob eine Person oder ein Agent es war. Das ganze Logbuch des
Kontos, das Neueste zuerst, steht unter Einstellungen → Logbook und in
`samu logbook`; ein Löschen, das anderes mitnimmt, ist dort ein Eintrag
mit der Anzahl. Löschen
lässt nichts in der Luft hängen: Die Aufgaben eines Projekts gehen in die
Inbox (oder werden mitgelöscht, wenn man es sagt), der Inhalt eines Ordners
rückt nach oben, eine Aufgabe nimmt ihre Kinder mit, und ein Tag wird
vorher von allem abgenommen. Das gelöschte Objekt bleibt als Grabstein
stehen, damit andere Geräte von der Löschung erfahren.

## Deine Daten

Alles lässt sich als Verzeichnis einfacher JSON-Dateien (und als
Org-Gliederung) exportieren, das man ohne samudo lesen kann, und in ein
leeres Konto zurückspielen. Siehe
[Selbst hosten](self-hosting.md#export-und-wiederherstellung).
