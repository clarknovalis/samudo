# Gewohnheiten

Eine Gewohnheit ist eine Aufgabe mit einem Ziel, das man Tag für Tag
einträgt: acht Gläser trinken, jeden Tag dehnen, dreimal die Woche
trainieren, die Finger von Zigaretten lassen. samudo zählt die Einträge
und rechnet Serie und Quote aus. Jeder Tap ist ein eigener Eintrag und
erledigt die Gewohnheit nicht. Entscheidungen und Gründe: ADR 0091
(Gewohnheiten) und ADR 0092 (die App).

## Eine Gewohnheit anlegen

Eine Aufgabe wird durch `habit:` in der Schnellerfassung zur Gewohnheit:

| Geschrieben | Bedeutet |
|---|---|
| `habit:` oder `habit:1/d` | einmal am Tag |
| `habit:3/w` | dreimal die Woche (Montag bis Sonntag) |
| `habit:2/m` | zweimal im Kalendermonat |
| `habit:8/d:glas` | eine Menge mit Einheit: acht Gläser am Tag |
| `habit:avoid` | etwas zum Vermeiden: Eingetragen werden nur Rückfälle |

```sh
samu add 'Wasser #Gewohnheiten @habit habit:8/d:glas'
samu habit set <id> 3/w        # eine bestehende Aufgabe zur Gewohnheit machen oder ihr Ziel ändern
samu habit set <id> none       # wieder eine einfache Aufgabe daraus machen
```

Mit einer [Vorlage](quick-capture.md#vorlagen) wird das eine einzige
Taste, zum Beispiel `samu template add habit '#Gewohnheiten @habit habit:1/d'`
mit `samu template key habit h`, dann `C h` im Web.

Eine Gewohnheit hat kein Fälligkeitsdatum und keine Wiederholung. Projekt,
Tags, Notiz, Scope und Links gehen wie bei jeder Aufgabe. **Eine
Gewohnheit zu erledigen oder zu verwerfen beendet sie.** Ihre Einträge
bleiben.

## Eintragen

Es gibt drei Arten von Einträgen:

- **geschafft:** einmal getan. Mit Einheit zählt jeder Eintrag 1, also für
  das nächste Glas noch einmal tippen. Auf der Kommandozeile trägt
  `--amount 2` zwei auf einmal ein.
- **nicht geschafft:** bewusst nicht getan. Das ist etwas anderes, als
  nichts einzutragen. Bei einer Gewohnheit zum Vermeiden ist es ein
  Rückfall.
- **Pause:** Urlaub oder Krankheit. Die Serie reißt nicht. Auf der
  Kommandozeile kann eine Pause mehrere Tage dauern.

Ein Eintrag gilt für heute oder einen Tag davor, nie für einen späteren
Tag. Ein falscher Eintrag wird gelöscht, nie geändert. Zwei Handys, die
offline je +1 tippen, ergeben zusammen 2.

### Web

- In jeder Liste zeigt die Zeile einen Knopf statt des Kreises: ✓ oder
  **+1** mit `5/8` daneben. Der Knopf wird grün, sobald das Ziel erreicht
  ist. Eine Gewohnheit zum Vermeiden zeigt „frei seit n T“ und einen Knopf
  **Rückfall**. Die kleine Zahl daneben ist die Serie.
- Das Kontextmenü hat „Heute geschafft“ (nicht bei einer Gewohnheit zum
  Vermeiden), „Heute nicht geschafft“ (bei einer Gewohnheit zum Vermeiden
  „Rückfall heute“) und „Pause heute“.
- Der Teil **Serie** in den Details zeigt das Ziel, die Serie, die
  längste Serie, die Quoten und einen Kalender der letzten 12 Wochen. Ein
  Klick auf einen Tag zeigt seine Einträge, löscht einen oder trägt für
  diesen Tag geschafft, nicht geschafft oder eine Pause ein.

### App

- **Zeile:** ✓ oder **+1** mit `5/8` und 🔥 mit der Serie. Eine Gewohnheit
  zum Vermeiden zeigt „frei seit n T“ und hat keinen Knopf.
- **Nach rechts wischen** heißt „heute geschafft“ und erledigt die
  Gewohnheit nie. Bei einer Gewohnheit zum Vermeiden tut Wischen nichts,
  damit ein Rückfall nicht aus Versehen passiert.
- **Langes Drücken** öffnet ein Menü: „Heute geschafft“ oder „+1 heute“,
  „Heute nicht geschafft“ (bei einer Gewohnheit zum Vermeiden „Rückfall
  heute“), „Pause heute“ und „Erledigen“, das die Gewohnheit beendet.
  Während man Aufgaben auswählt, wählt langes Drücken wie gewohnt.
- **Aufgabenseite, Abschnitt „Serie“:** dieselben Zahlen und derselbe
  Kalender wie im Web. Ein Tap auf einen Tag löscht einen Eintrag oder
  trägt einen für diesen Tag ein. Bei einer Gewohnheit zum Vermeiden bietet
  das Blatt des Tages **Frei ab hier** statt „Geschafft“: Es trägt für
  diesen Tag geschafft ein, sodass „frei seit“ von dort zählt, wenn der Tag
  vor dem Beginn der Gewohnheit liegt. Das ist die einzige Stelle in der
  App, die bei einer Gewohnheit zum Vermeiden geschafft einträgt.
- Alles geht offline über den Ausgang und zählt sofort.

### Widget „Habits“

„samudo · Habits“ auf den Startbildschirm ziehen. Es listet jede offene
Gewohnheit, in der Reihenfolge von Projekten und Positionen, auch
Gewohnheiten, die nur in ihrem eigenen Scope erscheinen.

- **✓ oder +1** trägt heute als geschafft ein. Die Zeile verblasst, sobald
  das Ziel erreicht ist.
- **Der Titel** öffnet die Aufgabe mit ihrem Abschnitt „Serie“, wo man
  einen falschen Tap zurücknimmt.
- **Eine Gewohnheit zum Vermeiden** zeigt „frei seit n T“ ohne Knopf. Einen
  Rückfall trägt man in der App ein.
- Neu gezeichnet wird es nach jedem Sync und um Mitternacht in der
  Zeitzone des Kontos, wenn ein neuer Tag beginnt.

Im Widget „Heute“ trägt der Kreis einer Gewohnheit ebenfalls heute als
geschafft ein und erledigt sie nicht.

### Emacs

Eine Zeile zeigt ✓, ✗, ○ oder die Menge wie `5/8 Glas`, und die Serie;
eine Gewohnheit zum Vermeiden zeigt „frei seit n T“. `h h` trägt geschafft
ein, `h f` nicht geschafft oder einen Rückfall, `h p` eine Pause, `h u`
löscht den letzten Eintrag, und `h s` zeigt die Serie, die Quoten, die
letzten 12 Wochen und die Einträge. Siehe [Emacs](emacs.md#gewohnheiten).

### Kommandozeile

```sh
samu habit list [--scope Habit]          # Stand heute und Serie
samu habit show <id> [--days 28]         # Serie, Quote, die letzten Tage als Zeichen
samu habit done <id> [--day D] [--amount N]
samu habit fail <id> [--day D]           # bei einer Gewohnheit zum Vermeiden: ein Rückfall
samu habit pause <id> [--from D] [--until D]
samu habit entries <id>
samu habit undo <eintrag>
```

`samu habit list` geht durch den gewählten Scope. Gewohnheiten, die nur in
ihrem eigenen Scope erscheinen, fehlen ohne ihn, also `--scope Habit`
nehmen.

## Wie gezählt wird

samudo rechnet das auf dem Server und in der App, offline:

- **Ein Tag** einer aufzubauenden Gewohnheit ist *geschafft*, sobald das
  Tagesziel erreicht ist, *teilweise* darunter, *nicht geschafft* mit einem
  solchen Eintrag, *Pause* in einer Pause und *nichts* ohne Eintrag. Bei
  einer Gewohnheit zum Vermeiden ist jeder Tag ohne Rückfall geschafft,
  gezählt ab dem Tag, an dem die Gewohnheit angelegt wurde.
- **Ein Zeitraum** (Tag, Woche oder Monat) ist erfüllt, sobald seine
  geschafften Mengen das Ziel erreichen.
- **Die Serie** zählt die erfüllten Zeiträume am Stück bis heute. Der
  laufende Zeitraum bricht sie nie, und einmal erfüllt zählt er. Ein
  verpasster Zeitraum mit mindestens einem Pausentag bricht nicht und zählt
  nicht. Nicht geschafft und kein Eintrag brechen sie beide. Bei einer
  Gewohnheit zum Vermeiden ist die Serie „frei seit n Tagen“: die Tage seit
  dem letzten Rückfall, ohne Pausen.
- **Die Quoten** der letzten 4 Wochen und dieses Jahres teilen die
  erfüllten Zeiträume durch die abgeschlossenen, ohne pausierte.
- **Für einen früheren Tag eintragen** als das Anlegen der Gewohnheit
  verlegt ihren Beginn nach vorn. Bei einer Gewohnheit zum Vermeiden legt
  das fest, ab wann „frei seit“ zählt: in der App mit **Frei ab hier** im
  Blatt des Tages, auf der Kommandozeile mit
  `samu habit done <id> --day 2026-10-01` oder im Web.

## Erinnerungen

Eine Gewohnheit erinnert mit einer **täglichen** Erinnerung: „täglich um“
in den Details oder in der App, `samu remind <id> --daily 20:00` oder
`daily@20:00` in einer Vorlage (so kann `/habit` eine mitbringen). Sie
meldet sich zu dieser Zeit in der Zone des Kontos, aber nur an einem Tag,
an dem noch etwas fehlt: nicht, sobald das Tagesziel erreicht ist, nicht
nach nicht geschafft oder in einer Pause, nicht, sobald ein Wochen- oder
Monatsziel erfüllt ist, und nie bei einer Gewohnheit zum Vermeiden. Wer
vor der Zeit einträgt, hält sie still. Wie jede Erinnerung klingelt sie auf
dem Handy.

## Noch nicht

Einiges davon folgt später, jeweils mit einem ADR:

- eine Grenze („höchstens n“),
- Minuten aus der Uhr,
- feste Wochentage,
- eine Notiz an einem Eintrag,
- eine Herausforderung mit Ende.
