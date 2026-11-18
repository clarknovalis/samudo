# Schnellerfassung

Eine Zeile wird zur Aufgabe: der Titel in normalen Worten, der Rest als
Zeichen. Derselbe Parser läuft auf dem Server und in der Android-App, also
bedeutet eine Zeile überall dasselbe: im Erfassen-Fenster des Webs, in der
App, in Emacs, im Desktop-Widget und auf der Kommandozeile.

```sh
samu add "Fliesenleger anrufen #Bad @telefon d:fr 9:00 ~15m !A ! // nach Fugenmasse fragen"
```

legt die Aufgabe *Fliesenleger anrufen* im Projekt *Bad* an, mit dem Tag
*telefon*, fällig nächsten Freitag um 9:00, geschätzt 15 Minuten, Priorität
A, markiert, mit der Notiz „nach Fugenmasse fragen“. Eine Zeile zuerst mit
`--dry-run` ausprobieren: Das zeigt, was angelegt würde, und schreibt
nichts. Im Web leuchten die Zeichen beim Tippen auf, und Warnungen
erscheinen unter dem Feld.

## Zeichen

| Zeichen | Bedeutung |
|---|---|
| `#Projekt` | das Projekt, mit Namen oder einem eindeutigen Anfang davon (`#ba` findet *Bad*); `#"Neue Wohnung"` für einen Namen mit Leerzeichen. Unbekannt oder mehrdeutig: Die Aufgabe geht in die Inbox, `#…` bleibt im Titel, mit einer Warnung |
| `@tag` | ein Tag; mehrere erlaubt. `@Ort/Büro` ist der Tag *Büro* unter *Ort*; `@"zwei Wörter"` mit Leerzeichen. Ein unbekannter Tag wird angelegt |
| `!` | markieren |
| `!A` `!B` `!C` | Priorität |
| `d:` oder `due:` | Fälligkeitsdatum |
| `s:` oder `start:` | Startdatum (vorher nicht verfügbar) |
| `p:` oder `plan:` | geplantes Datum |
| `~15m` | Schätzung: `30m`, `45min`, `2h`, `1h30`, `1h30m` |
| `every:` | Wiederholung, fest: `every:1d`, `every:2w`, `every:1m`, `every:1y`, `every:mon` |
| `every+:` | Wiederholung, fest, verpasste Termine bis heute übersprungen |
| `every!:` | Wiederholung, Fälligkeit ab dem Tag des Erledigens gezählt |
| `habit:` | macht eine [Gewohnheit](habits.md) daraus: `habit:` (täglich), `habit:3/w`, `habit:2/m`, `habit:8/d:glas`, `habit:avoid` |
| `z:` | schon verbrachte Zeit buchen: `z:1h30` (endet jetzt), `z:gestern 2h`, `z:14-15:30`, `z:gestern 14:00-15:30` |
| `x:` | gleich als erledigt anlegen: `x:` (jetzt), `x:gestern` |
| `/name` | eine [Vorlage](#vorlagen) anwenden |
| `// text` | alles danach ist die Notiz |
| `\#5` | ein Backslash lässt das nächste Wort Text bleiben |

Jedes Zeichen darf einmal vorkommen (Tags ausgenommen); ein zweites
Projekt, Datum, eine zweite Schätzung oder Wiederholung bleibt mit einer
Warnung im Titel.

## Daten

Ein Datum wird nur nach einem Präfix gelesen (`d:`, `s:`, `p:`, …), nie
frei im Satz, und es wird nie geraten: Ein Wert, der kein Datum ist, bleibt
mit einer Warnung im Titel. Deutsche und englische Wörter gehen beide.

| Geschrieben | Bedeutet |
|---|---|
| `heute` `today`, `morgen` `tomorrow`, `übermorgen` | |
| `mo` … `so`, `montag` …, `mon` … `sun`, `monday` … | der nächste solche Wochentag, nie heute |
| `nächste woche` `next week` | nächster Montag |
| `monatsende` `end of month` | der letzte Tag dieses Monats |
| `in 3 tagen`, `in 2 wochen`, `in 1 monat` (auch `days`, `weeks`, `months`, `jahre`) | ab heute gezählt |
| `+3d`, `+2w`, `+1m`, `+1y` | dasselbe, kurz |
| `2026-12-24` | ein volles Datum |
| `24.12.` oder `24.12.2026` | der nächste 24. Dezember (heute eingeschlossen) oder der dieses Jahres |

Eine Uhrzeit nach dem Datum macht daraus einen Zeitpunkt in der Zeitzone
des Kontos: `d:fr 9:00`, `p:morgen 14:30`, `s:mo 9h`. Ohne Uhrzeit ist das
Datum ein ganzer Tag.

Bei `z:` und `x:` liegt der Tag in der Vergangenheit: `heute`, `gestern`,
`vorgestern`, ein Wochentag (der letzte, heute eingeschlossen), `t.m.` oder
ein volles Datum. Ein `z:` ohne Zeitspanne schließt an die letzte Uhr
dieses Tages an, sonst ab 09:00.

## Mit Zeichen bearbeiten

`samu edit <id> <zeichen>` (Taste `e` im Web und in Emacs) wendet eine
Zeile nur aus Zeichen auf eine bestehende Aufgabe an:

```sh
samu edit 4f2a91c3 "@besorgung !A d:mo // nach dem Preis fragen"
```

Tags kommen hinzu (ein Zustand eines Ablaufs ersetzt seinen Geschwister),
Projekt, Priorität, Datum, Schätzung, Wiederholung und Markierung
überschreiben, `// text` wird an die Notiz angehängt, `/name` wendet eine
Vorlage an. Freier Text wird abgelehnt, denn der Titel hat sein eigenes
Feld: `samu set <id> title "Neuer Titel"`.

## Mehrere Zeilen

Ein Text aus mehreren Zeilen ergibt eine Aufgabe je Zeile, jede Zeile mit
ihren eigenen Zeichen und ihrer `// Notiz`. Leere Zeilen fallen weg, und
Listenzeichen am Zeilenanfang werden entfernt: `-`, `*`, `+`, `•`, `1.`,
`1)` und ein Kästchen `[ ]` oder `[x]`. Eine Zeile, die tiefer eingerückt
ist als die vorige, kommt als Unteraufgabe darunter:

```text
Einkauf #Zuhause
  - Milch @laden
  - Brot
Fliesenleger anrufen d:fr
```

ergibt vier Aufgaben, „Milch“ und „Brot“ unter „Einkauf“. Angelegt werden
alle auf einmal oder keine: Scheitert eine Zeile, etwa an einem Projekt,
in das die Sitzung nicht schreiben darf, entsteht keine. Einen solchen Text
fügt man ins Erfassen-Fenster im Web oder ins Erfassen-Feld der App ein,
oder man schaltet auf „Mehrere Zeilen“ und tippt sie (im Web auch mit
Umschalt+Enter, in der App auch „Mehrere erfassen“ im ⋮-Menü). Auf der
Kommandozeile nimmt `samu add --lines -` den Text von stdin. In Emacs
nimmt `samu-capture-lines` die Region, ohne Region öffnet es einen eigenen
Puffer. Ein in die App geteilter Text bleibt eine Aufgabe.

## Vorlagen

Eine Vorlage ist eine Zeile aus Zeichen unter einem Namen, mit wahlweise
Erinnerungen und einer Taste:

```sh
samu template add habit '#Gewohnheiten @habit habit:1/d'
samu template add abend '#Zuhause p:heute 21:00 every+:1d'
samu template key abend e          # Taste e nach der Erfassen-Taste: C e im Web
samu add "Blumen gießen /abend"
```

Freier Text in einer Vorlage steht vor dem Titel des Erfassten: Mit
`samu template add epic '[EPIC] @epic'` wird aus der Zeile `Doku
veröffentlichen /epic` die Aufgabe `[EPIC] Doku veröffentlichen`. Auf eine
bestehende Aufgabe angewendet, lässt eine Vorlage deren Titel in Ruhe.

Relative Daten in einer Vorlage zählen ab dem Moment des Erfassens. Die
Zeile gewinnt Feld für Feld: Was sie nennt, überschreibt die Vorlage, Tags
werden nicht gemischt. Die Erfassen-Einstellungen eines Scopes füllen nur,
was beide offen lassen.

## Scope und Ort

Ist ein [Scope](concepts.md#scopes) gewählt, geht eine Zeile ohne Projekt
in das Projekt der Ansicht, in der man ist, sonst in das Erfassen-Projekt
des Scopes; eine Zeile ohne Tags bekommt die Erfassen-Tags des Scopes. Was
die Zeile nennt, gewinnt immer. `samu add --no-scope` lässt den Scope
weg; `--parent <id>` legt die neue Aufgabe unter eine andere. Bei
mehreren Zeilen gilt `--parent` für die oberen.
