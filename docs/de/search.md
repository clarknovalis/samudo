# Suchsprache

Eine kleine Sprache filtert Aufgaben überall: das Suchfeld der
Web-Oberfläche und der App, der Filter eines Scopes und eine Perspektive.
Sie besteht aus **freiem Text** und **Chips**. Jedes Wort der Sprache hat
eine deutsche und eine englische Form, und beide werden verstanden, gleich
welche Sprache die Oberfläche spricht (siehe
[Deutsch und Englisch](#deutsch-und-englisch)).

Auf der Kommandozeile nimmt `samu list --search <wörter>` nur freien Text
und lehnt einen Chip ab (Exit 2); in doppelten Anführungszeichen
(`--search '"#bad"'`) wird er als Text gesucht. Chips gehören in Scopes (`samu scope filter`) und Perspektiven
(`samu perspective add`), und `samu list --perspective <name>` zeigt eine.

## Freier Text

Jedes Wort, das kein Chip ist, muss im Titel, in der Notiz oder in der ID
vorkommen. Ein Wort, das eine Aufgabe über ihre ID nennt, findet diese
Aufgabe, was auch immer der Filter der Ansicht zeigt, auch erledigt oder
verworfen; die anderen Aufgaben, die die Wörter finden, halten sich an
den Filter. So ein Wort ist das Ende einer ID ab acht Zeichen (die
Kommandozeile gibt acht aus), die ganze ID, ein kopierter Link
`samu:<id>` oder die Web-Adresse einer geöffneten Aufgabe.

Ein Wort mit `-` davor darf nicht vorkommen: `Milch -Hafer`. Wörter in
doppelten Anführungszeichen sind eine Phrase und müssen zusammen und in
dieser Reihenfolge vorkommen: `"bis Freitag"`; `-"nicht heute"` lässt
weg, was sie enthält. Wer das Wort `oder` selbst sucht, setzt es in
Anführungszeichen: `"oder"` (siehe [Oder](#oder)).

## Chips

Ein Wort wird zum Chip, sobald man es aus der Liste unter dem Feld nimmt
(im Web ↑ ↓, Tab oder Enter; in der App durch Tippen) oder mit einem
Leerzeichen abschließt; im Web schließt Enter es auch ab, wenn die Liste
nichts anbietet, wie bei `id:…`. Beim Tippen bietet die Liste an, was zum Wort
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
| `ist:` + Wort | `markiert`, `verfügbar`, `wartet`, `offen`, `erledigt`, `verworfen`, `gruppe` (Aktionsgruppe), `notiz` (hat eine Notiz), `wiederholt`, `geclockt` (hat Uhren), `blocker` (eine andere offene Aufgabe wartet auf sie), `blockiert` (wartet auf einen offenen Blocker, eigenen oder den ihres Projekts), `versteckt` (offen, aber unter einer erledigten oder verworfenen Aufgabe; siehe [Konzepte](concepts.md#erledigen-mit-offenen-aufgaben-darunter)) |
| `ist:termin` | Termine statt Aufgaben, nach Titel, Ort und Notiz; ein wiederholter einmal, mit seinem nächsten Vorkommen; Datumschips gelten für den Beginn (nur im Web, nicht im Filter eines Scopes) |
| `~<15m`, `~>=1h`, `~30m`, `~mit`, `~ohne` | nach Schätzung (`<`, `<=`, `>`, `>=`, gleich; mit oder ohne) |
| `prio:a`, `prio:b`, `prio:c`, `prio:mit`, `prio:ohne` (oder `prio:-`) | nach Priorität |
| `prop:jira`, `prop:jira=ABC-1` | mit der Property, eigen oder geerbt; mit Wert, ohne auf Groß- und Kleinschreibung zu achten |
| `id:1a2b3c4d`, `id:` + ganze ID | die Aufgabe mit dieser ID, über ihr Ende ab acht Zeichen (die Kommandozeile gibt acht aus), die ganze ID, einen Link `samu:<id>` oder die Web-Adresse der geöffneten Aufgabe; `id:1a2b3c4d id:5e6f7a8b` findet beide, so wird eine Suche zur Liste von Aufgaben |

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

Wer nach erledigten Aufgaben fragt (`ist:erledigt`, `erledigt:…`) oder
nach einer Aufgabe über ihre ID (`id:…`), weitet die Ansicht auf „Alle“; wer nach wartenden fragt (`ist:wartet`,
`start:…`), weitet „Verfügbar“ auf „Verbleibend“. Das gilt, sobald
irgendein Chip danach fragt, in welcher Gruppe auch immer. Ein freies
Wort, das eine Aufgabe über ihre ID nennt, weitet die Ansicht nur für
diese Aufgabe.

## Oder

Das Wort `oder` (englisch `or`) zwischen Chips teilt sie in **Gruppen**;
gefunden wird eine Aufgabe, die eine Gruppe erfüllt:

- `ist:markiert oder fällig:heute` – markiert oder heute fällig.
- `#Website oder @telefon -ist:wartet` – im Projekt Website, oder mit
  @telefon und nicht wartend.

Innerhalb einer Gruppe gilt alles wie oben. Freier Text gilt für jede
Gruppe: `ist:markiert oder fällig:heute Rechnung` findet die markierten
und die fälligen Aufgaben, in denen „Rechnung“ steht. Derselbe Chip darf
in zwei Gruppen stehen.

`oder` wird im Feld ein eigener Chip und steht in der Liste darunter zur
Wahl, sobald ein Chip da ist. Ein neuer Chip kommt an die Einfügestelle,
am Ende, solange man sie nicht verschiebt: im Web rückt ← am Anfang des
Felds sie über die Chips zurück und → wieder vor, Backspace dort
entfernt den Chip davor; ein Klick (in der App Tippen) auf ein `oder`
setzt sie davor, der nächste Chip kommt also in die Gruppe davor, ein
zweiter Klick zurück ans Ende. Ein `oder` mitten in einer Gruppe teilt
sie.
Wer `oder` entfernt, verbindet seine beiden Gruppen. Ein `oder` am
Anfang oder doppelt bedeutet nichts und fällt weg; eines am Ende wartet
auf den nächsten Chip. Gruppen
lassen sich nicht verschachteln, und innerhalb einer Art gibt es kein
„und“: `@a @b` bleibt eines der beiden Tags.

Scopes und Perspektiven behalten die Gruppen, und die Kommandozeile nimmt
und zeigt sie genauso: `samu scope filter Arbeit "ist:markiert oder fällig:heute"`.

## Deutsch und Englisch

| Deutsch | Englisch |
|---|---|
| `fällig:` `geplant:` `start:` `erledigt:` `erstellt:` `geändert:` | `due:` `planned:` `defer:` `done:` `created:` `modified:` |
| `ordner:` `zustand:` `ist:` | `folder:` `state:` `is:` |
| `heute` `morgen` `gestern` `woche` `überfällig` `mit` `ohne` | `today` `tomorrow` `yesterday` `week` `overdue` `any` `none` |
| `7t` `-30t` | `7d` `-30d` |
| `ist:markiert` `verfügbar` `wartet` `offen` `erledigt` `verworfen` `gruppe` `notiz` `wiederholt` `geclockt` `blocker` `blockiert` `versteckt` `termin` | `is:flagged` `available` `waiting` `open` `done` `dropped` `group` `note` `repeating` `clocked` `blocker` `blocked` `hidden` `appointment` |

`#`, `@`, `~`, `prio:`, `prop:` und `id:` sind in beiden gleich. Ein Chip wird
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
