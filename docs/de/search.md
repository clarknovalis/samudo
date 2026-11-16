# Suchsprache

Eine kleine Sprache filtert Aufgaben überall: das Suchfeld der
Web-Oberfläche und der App, der Filter eines Scopes und eine Perspektive.
Sie besteht aus **freiem Text** und **Chips**. Jedes Wort der Sprache hat
eine deutsche und eine englische Form, und beide werden verstanden, gleich
welche Sprache die Oberfläche spricht (siehe
[Deutsch und Englisch](#deutsch-und-englisch)).

Auf der Kommandozeile nimmt `samu list --search <wörter>` nur freien Text;
Chips gehören in Scopes (`samu scope filter`) und Perspektiven
(`samu perspective add`), und `samu list --perspective <name>` zeigt eine.

## Freier Text

Jedes Wort, das kein Chip ist, muss im Titel, in der Notiz oder in der ID
vorkommen. Die letzten acht Zeichen einer ID, wie die Kommandozeile sie
ausgibt, finden ihre Aufgabe.

## Chips

Ein Wort wird zum Chip, sobald man es aus der Liste unter dem Feld nimmt
(im Web ↑ ↓, Tab oder Enter; in der App durch Tippen) oder mit einem
Leerzeichen abschließt. Beim Tippen bietet die Liste an, was zum Wort
passt: die Arten, dann Projekte, Tags, Ordner, Zustände und die Werte
eines Datums. Bei leerem Feld zeigt das Web die Arten; die App zeigt
schnelle Filter, die letzten Suchen und die Perspektiven (siehe
[Android-App](android.md#suche)).

| Chip | Findet Aufgaben |
|---|---|
| `#Website`, `#Inbox` | im Projekt oder in der Inbox |
| `@Ort`, `@Ort/Büro` | mit dem Tag oder einem Tag darunter |
| `ordner:Arbeit` | in Projekten des Ordners und seiner Unterordner |
| `zustand:"In Arbeit"` | in diesem Zustand ihres Ablaufs |
| `fällig:`, `geplant:`, `start:` + Bereich | mit diesem Datum im Bereich (siehe unten) |
| `erledigt:`, `erstellt:`, `geändert:` + Bereich | im Bereich erledigt, angelegt oder geändert |
| `ist:` + Wort | `markiert`, `verfügbar`, `wartet`, `offen`, `erledigt`, `verworfen`, `gruppe` (Aktionsgruppe), `notiz` (hat eine Notiz), `wiederholt`, `geclockt` (hat Uhren), `blocker` (eine andere offene Aufgabe wartet auf sie), `blockiert` (wartet auf einen offenen Blocker, eigenen oder den ihres Projekts) |
| `ist:termin` | Termine statt Aufgaben, nach Titel, Ort und Notiz; ein wiederholter einmal, mit seinem nächsten Vorkommen; Datumschips gelten für den Beginn (nur im Web, nicht im Filter eines Scopes) |
| `~<15m`, `~>=1h`, `~30m`, `~mit`, `~ohne` | nach Schätzung (`<`, `<=`, `>`, `>=`, gleich; mit oder ohne) |
| `prio:a`, `prio:b`, `prio:c`, `prio:mit`, `prio:ohne` (oder `prio:-`) | nach Priorität |
| `prop:jira`, `prop:jira=ABC-1` | mit der Property, eigen oder geerbt; mit Wert, ohne auf Groß- und Kleinschreibung zu achten |

### Bereiche

| Bereich | Bedeutet |
|---|---|
| `heute`, `morgen`, `gestern` | heute, morgen, gestern |
| `woche` | heute bis Sonntag |
| `7t`, `30t` | heute und die nächsten 7 oder 30 Tage |
| `-7t`, `-30t` | die letzten 7 oder 30 Tage bis heute |
| `überfällig` | überfällig |
| `mit`, `ohne` | hat das Datum, hat keins |
| `2026-11-01` | dieser Tag |
| `2026-11-01..2026-11-30`, `..2026-11-30`, `2026-11-01..` | eine Spanne, auf Wunsch an einem Ende offen |

Tage zählen in der Zeitzone des Kontos.

## Kombinieren

- Chips **derselben Art** sind Alternativen: `@telefon @rechner` findet
  beides.
- Chips **verschiedener Art** müssen alle gelten: `@telefon fällig:woche`.
- `-` vor einem Chip verneint ihn: `-@Wartet`. Ein Klick (in der App
  Tippen) auf einen Chip schaltet die Verneinung um, × entfernt ihn; im
  Web entfernt Backspace im leeren Feld den letzten, Escape leert alles.

Wer nach erledigten Aufgaben fragt (`ist:erledigt`, `erledigt:…`), weitet
die Ansicht auf „Alle“; wer nach wartenden fragt (`ist:wartet`,
`start:…`), weitet „Verfügbar“ auf „Verbleibend“.

## Deutsch und Englisch

| Deutsch | Englisch |
|---|---|
| `fällig:` `geplant:` `start:` `erledigt:` `erstellt:` `geändert:` | `due:` `planned:` `defer:` `done:` `created:` `modified:` |
| `ordner:` `zustand:` `ist:` | `folder:` `state:` `is:` |
| `heute` `morgen` `gestern` `woche` `überfällig` `mit` `ohne` | `today` `tomorrow` `yesterday` `week` `overdue` `any` `none` |
| `7t` `-30t` | `7d` `-30d` |
| `ist:markiert` `verfügbar` `wartet` `offen` `erledigt` `verworfen` `gruppe` `notiz` `wiederholt` `geclockt` `blocker` `blockiert` `termin` | `is:flagged` `available` `waiting` `open` `done` `dropped` `group` `note` `repeating` `clocked` `blocker` `blocked` `appointment` |

`#`, `@`, `~`, `prio:` und `prop:` sind in beiden gleich. Ein Chip wird
**in seiner deutschen Form gespeichert**: Ein Scope oder eine Perspektive
behält `heute`, `markiert`, `7t`, und die Oberfläche zeigt ihn in ihrer
Sprache. So liest sich eine auf Englisch angelegte Perspektive auf einem
deutschen Handy genauso, und die Kommandozeile, die deutsch schreibt,
nimmt beide.

## Anordnen

Der Knopf „Anordnen“ im Web gruppiert (nach Projekt, Ordner, Tag, Zustand,
Priorität, Datum oder gar nicht) und sortiert (nach eigener Reihenfolge,
fällig, geplant, Start, Titel, angelegt, geändert, Schätzung oder
Priorität, auf- oder absteigend), mit einem zweiten Schlüssel für
Gleichstände. Die Wahl bleibt je Ansicht im Browser. Eine Perspektive
speichert die Suche zusammen mit Filter, Anordnung und Liste oder Board.

```sh
samu perspective add "Kurze Anrufe" '@telefon ~<15m'
samu perspective arrange "Kurze Anrufe" --help     # Gruppierung und Sortierung
samu list --perspective "Kurze Anrufe"
```
