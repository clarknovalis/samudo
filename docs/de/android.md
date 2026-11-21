# Android-App

Eine native App (Kotlin und Jetpack Compose) auf demselben Rust-Kern wie
der Server, sodass Verfügbarkeit, Schnellerfassung, Uhren und
Gewohnheiten auf dem Handy genauso berechnet werden, auch offline. Die Oberfläche spricht Deutsch oder Englisch (siehe
[Sprache](#sprache)). Wie man sie baut, steht in [`android/README.md`](../android/README.md).

## Installieren

Die App erscheint unter
[GitHub Releases](https://github.com/clarknovalis/samudo/releases):
`samudo-<version>.apk` aus dem neuesten Release herunterladen und auf dem
Handy öffnen (Android 8 oder neuer, 64-Bit-ARM). Jedes Release ist mit
demselben Schlüssel signiert; eine neuere Version installiert sich also
über die alte und behält ihre Daten.

**Updates mit [Obtainium](https://github.com/ImranR98/Obtainium):** „App
hinzufügen“, die URL `https://github.com/clarknovalis/samudo`,
„Hinzufügen“. Obtainium prüft dann die Releases und bietet jede neue
Version an.

**Die Signatur prüfen**, wer mag, vor der ersten Installation: SHA-256 des
Zertifikats ist

```
04:72:BC:07:C9:29:ED:F6:2E:43:1E:23:77:2F:01:C4:63:41:20:2D:98:A0:D0:A0:74:37:FB:9B:AC:80:F3:AF
```

`apksigner verify --print-certs samudo-<version>.apk` gibt ihn aus (hex
ohne Doppelpunkte), und [AppVerifier](https://github.com/soupslurpr/AppVerifier)
vergleicht ihn auf dem Handy.

**Von einer selbst gebauten App** vor Release 0.1.0, die den eigenen
Debug-Schlüssel trug: Android installiert nicht über eine andere
Signatur. Erst den Ausgang leeren (Sync; das Menü ⋮ zeigt, was wartet),
die alte App deinstallieren, das Release installieren und neu anmelden;
Widgets müssen neu angelegt werden.

**Selbst bauen** ([`android/README.md`](../android/README.md)): ein
Debug-Build heißt „samudo dev“ mit dem Paket `de.samudo.app.dev`. Er läuft
neben der Release-App, mit eigenem Cache und eigener Anmeldung; die
Broadcasts für [Tasker](#automatisieren-mit-tasker) gehen weiter an
`de.samudo.app`.

## Anmelden

Entweder mit Server, Konto, Passwort und einem Gerätenamen oder per
**Koppeln**: in der Web-Oberfläche das Kontomenü öffnen, „Handy koppeln“,
und den QR-Code mit der App scannen (oder `samu pair phone` auf der
Kommandozeile). Der Code gilt einmal, für zehn Minuten. Die App behält nur
das Sitzungs-Token.

## Offline zuerst

Jede Änderung geht zuerst in den **Ausgang** und wird mit dem nächsten
Sync gesendet: beim Öffnen der App, nach jeder Änderung, wenn man eine
Liste herunterzieht, und alle 15 Minuten, solange es Netz gibt. Ein Punkt
am Titel zeigt, dass etwas wartet; das Menü (⋮) sagt, wie viel. Dateien
warten auf dem Handy und gehen vor den Änderungen hoch, die auf sie
verweisen. Nur Löschen braucht eine Verbindung.

## Der Kopf

Eine Zeile: der Titel, der [Scope](concepts.md#scopes) als Chip (Tippen
wählt einen anderen) und ein Menü (⋮) mit „Zeile …“, „Sync“,
„Einstellungen“ und, in den Projekten, „Neues Projekt“ oder Ordner.
„Zeile …“ ordnet die Felder der Zeile einer Aufgabe wie im
[Web](web.md#liste): Zeile 1 mit dem Titel, Zeile 2 und „Aus“ als
Reihen von Chips; Tippen auf einen Chip verschiebt ihn nach links,
rechts oder in eine andere Reihe. Das Handy hat seine eigene Wahl und
beginnt mit dem Zustand vor dem Titel, Fällig dahinter und Tags
darunter, kurz.
**Eine Liste herunterziehen** synct. Die Tag-Chips sind die erste Zeile
der Liste und scrollen mit ihr weg; jeder filtert nach einem Tag und
allem darunter.

## Die Leiste

Die Leiste unten zeigt fünf Einträge auf einmal; seitlich wischen zeigt
weitere, ein Punkt am Rand sagt, dass es weitergeht. Ohne Einstellung hat
sie **Inbox**, **Verfügbar**, **Forecast**, **Projekte** (ein Baum aus
Ordnern und Projekten, die sich anlegen, verschieben und zuklappen lassen)
und **Suche**.

Einstellungen › „Leiste“ bestimmt, was darin steht, nur auf diesem
Handy: einen Eintrag am Griff ziehen, mit seinem Schalter ausblenden, mit
× entfernen. „Hinzufügen“ bietet die festen Ansichten (dazu **Heute**,
**Markiert**, **Stuck**, **Review** und **Habits**), die eigenen
Perspektiven, Projekte (Suche nach Name) und Tags. Die Inbox lässt sich
nur verschieben. Ein gelöschtes oder verworfenes Projekt, ein gelöschter
Tag oder eine gelöschte Perspektive verschwindet von selbst aus der
Leiste.

- Eine **Perspektive** zeigt ihre Aufgaben als Liste, mit Scope und
  Filter wie im Web; ihre Chips stehen unter dem Titel, zum Ansehen.
  Ändern bleibt im Web und auf der [Kommandozeile](cli.md). In der Leiste
  trägt sie ihr Emoji, ohne eines einen Trichter.
- Ein **Projekt** öffnet wie aus dem Baum, ein **Tag** zeigt die
  verfügbaren Aufgaben mit ihm.

## Suche

Die Suche nimmt die [Suchsprache](search.md), auf Deutsch oder Englisch.
Beim Tippen bietet die Liste unter dem Feld Arten, Projekte, Tags, Ordner,
Zustände und Daten für das Wort am Cursor an, auch offline; Tippen nimmt
einen Vorschlag. Ein erkanntes Wort wird ein **Chip im Feld**: Tippen
verneint ihn, × entfernt ihn.

Das leere Feld zeigt fünf schnelle Filter („Markiert“, „Heute fällig“,
„Überfällig“, „Ohne Projekt“, „Heute erledigt“), die letzten fünf Suchen
auf diesem Handy und die eigenen Perspektiven. Mit mindestens einem Chip
oder Wort sichert „☆ Als Perspektive“ die Suche als Perspektive (Emoji
und Name) und öffnet sie.

## Mit Aufgaben arbeiten

- **Zeilen** haben einen runden Haken; nach rechts wischen oder auf den
  Haken tippen erledigt (oder öffnet wieder). **Nach links wischen**
  markiert oder hebt die Markierung auf. Auf eine Zeile tippen öffnet die
  Aufgabe.
- **Rückgängig:** Nach Erledigen, Markieren, Entmarkieren oder Verwerfen
  aus einer Liste sagt eine Leiste unten, was geschah („3 erledigt“ bei
  einer Auswahl), und bietet ein paar Sekunden lang „Rückgängig“ an.
  Solange sie steht, wartet die Änderung auf dem Handy; Rückgängig nimmt
  sie wieder aus dem Ausgang, der Server sieht sie nie (kein Eintrag im
  Logbuch, keine nächste Aufgabe einer Wiederholung). Hat ein Sync sie
  schon gesendet, schreibt Rückgängig stattdessen die Gegenänderung.
- **Offene Aufgaben darunter:** Eine Aufgabe mit offenen Aufgaben darunter
  zu erledigen oder zu verwerfen, auf welchem Weg auch immer, fragt zuerst
  in einem Sheet: „Mit erledigen“, „Herausholen“ oder „Abbrechen“ (siehe
  [Konzepte](concepts.md#erledigen-mit-offenen-aufgaben-darunter)), für
  eine ganze Auswahl einmal. Das geht auch offline; „Rückgängig“ nimmt
  alles zurück, was die Antwort geändert hat, auch herausgeholte Aufgaben.
- **Langes Drücken** öffnet das Menü der Aufgabe: „Bearbeiten“,
  „Unteraufgabe anlegen“, „Markieren“, „Zurückstellen bis …“,
  „Verschieben …“ (in die Inbox oder ein Projekt, Kinder gehen mit) und
  „Auswählen“. Eine Gewohnheit behält ihr eigenes Menü.
- **Bearbeiten** zeigt den Titel in einem eigenen Feld, was gesetzt ist,
  als Chips (✕ merkt einen zum Entfernen vor, ↶ nimmt das zurück) und
  eine Zeile in der Syntax der [Schnellerfassung](quick-capture.md) mit
  Vorschlägen und einer Vorschau dessen, was sie hinzufügt. „Sichern“
  schreibt alles auf einmal, auch offline.
- **Viele auswählen:** „Auswählen“ im Menü einer Aufgabe oder im Menü (⋮)
  des Kopfs startet die Auswahl; Tippen nimmt dann Aufgaben dazu oder
  heraus, bis ✕ oder Zurück. Die Leiste bietet erledigen, markieren,
  verschieben und unter „⋯“ wieder öffnen, verwerfen, planen, Daten,
  Tags, Zustand und löschen; bei genau einer Aufgabe auch bearbeiten und
  die Uhr.
- **Die Aufgabenseite** ist bearbeitbar: Titel, Notiz, Start-, Fälligkeits-
  und Plandatum, Schätzung, Tags und Zustand als Chips, die Projektzeile
  verschiebt die Aufgabe, dazu Wiederholung und Erinnerungen.
- **Kommentare und Anhänge** unten: Text, ein **Foto** aus der Kamera-App,
  eine **Aufnahme** („● Aufnehmen“, fragt beim ersten Mal nach dem
  Mikrofon) oder eine beliebige **Datei**. Mit Text wird daraus ein
  Kommentar mit den Dateien daran; ohne hängen die Dateien an der Aufgabe.
  Bilder zeigen eine Vorschau, Aufnahmen spielen ab, andere Dateien öffnen
  in einer anderen App. Unter einer Aufnahme steht ihr Transkript; auf
  einem Server mit [Transkript-Dienst](self-hosting.md#transkripte) bittet
  „Transkribieren“ darum.
- **Uhr** („Zeit“): Summe gegen die Schätzung, Start/Stopp und die letzten
  Uhren. Eine laufende Uhr steht als dauerhafte Benachrichtigung mit Zeit
  und „Stopp“ da.
- **Gewohnheiten:** siehe [Gewohnheiten](habits.md#app).

## Erfassen und Teilen

- **Erfassen** öffnet ein Blatt mit der
  [Schnellerfassung](quick-capture.md); die Vorschau rechnet das Handy,
  auch offline. Mit [mehreren Zeilen](quick-capture.md#mehrere-zeilen),
  eingefügt oder nach dem Schalter „Mehrere Zeilen“ bzw. „Mehrere erfassen“
  im ⋮-Menü, beginnt Enter eine neue Zeile, die Vorschau zeigt eine Aufgabe
  je Zeile, und „N Aufgaben anlegen“ legt alle an.
- **An samudo teilen** aus jeder App, die Text teilt: Betreff oder erste
  Zeile werden der Titel (bearbeitbar), darunter ein Feld der
  Schnellerfassung für Projekt, Tags und Daten. Der Rest des Textes wird
  die Notiz, und jede Adresse mit `https://`, `mid:` oder `catastr:` ein
  Link. Ohne Sitzung wartet der geteilte Text, bis man sich anmeldet.
- **`samu:<id>`-Links** in anderen Apps öffnen die Aufgabe.

## Sprachnotizen

Der runde Knopf „●“ neben „+ Erfassen“ öffnet die Aufnahme, ebenso
„Sprachnotiz“ nach langem Druck auf das Symbol der App. Sie nimmt vom
ersten Moment an auf (beim ersten Mal fragt sie nach dem Mikrofon).

- **„■ Stopp und anlegen“** legt eine Aufgabe „Sprachnotiz *Datum
  Uhrzeit*“ mit der Aufnahme in die Inbox, auch offline. Auf einem Server
  mit [Transkript-Dienst](self-hosting.md#transkripte) wird das
  Gesprochene danach zum Titel (die erste Zeile) und zur Notiz, wenn du
  beides inzwischen nicht geändert hast. Der am Handy gewählte Scope
  füllt auf, was er neuen Aufgaben mitgibt.
- **„Aufnahme verwerfen“** behält nichts.
- **„Weitergeben …“** stoppt und gibt die Aufnahme an eine andere App
  deiner Wahl, für das, was doch keine Aufgabe ist; samudo behält davon
  nichts.
- Wer die Aufnahme verlässt, während sie läuft (Home, Bildschirm aus),
  legt das Gesagte an; nichts geht verloren.
- **An samudo geteiltes Audio** aus einer anderen App, etwa einem
  Diktiergerät, wird genauso eine Sprachnotiz.
- **Als digitaler Assistent:** unter Einstellungen → Apps → Standard-Apps
  → Digitale Assistenz-App samudo wählen. Die Taste oder Geste, die den
  Assistenten ruft (auf den meisten Handys ein langer Druck auf die
  Seiten- oder Home-Taste), öffnet dann die Aufnahme, auch über dem
  Sperrbildschirm, wo sonst nichts von der App zu sehen ist. Android
  vergibt diese Rolle an eine App zur Zeit; sonst tut samudo nichts, was
  ein Assistent tut.
- **Auf einer anderen Taste:** der Shortcut „Sprachnotiz“ ist eine
  App-Aktion; was eine App-Aktion ausführen kann, öffnet also die
  Aufnahme. Auf einem Samsung-Handy etwa: eine Routine in „Modi und
  Routinen“, die manuell gestartet wird und samudos Aktion „Sprachnotiz“
  ausführt, gewählt für den Doppeldruck der Seitentaste (Einstellungen →
  Erweiterte Funktionen → Seitentaste). Die Assistenten-Taste bleibt
  dann für eine andere App frei.

## Widgets

- **„Heute“**: überfällig, heute, dann markierte verfügbare Aufgaben. Der
  Haken erledigt über den Ausgang, „+“ öffnet das Erfassen, eine Zeile
  öffnet die Aufgabe. Bei einer Aufgabe mit offenen Aufgaben darunter
  öffnet der Haken sie stattdessen. Neu gezeichnet nach jedem Sync und beim Verlassen der
  App.
- **„Habits“**: jede offene Gewohnheit; siehe
  [Gewohnheiten](habits.md#widget-habits).

**Für einen Launcher.** In denselben Momenten meldet die App der
eingestellten Start-App, wie Eingang und Tag stehen, als expliziter
Broadcast `de.samudo.app.action.GLANCE` mit den ganzzahligen Extras
`inbox`, `overdue`, `today_open` und `today_done`; abgemeldet fehlen sie.
Ein Launcher, der einen exportierten Empfänger für die Aktion anmeldet,
kann die Zahlen zeigen. Ab Android 14 nennt der Broadcast seinen
Absender, den der Empfänger mit `getSentFromPackage()` prüfen kann.

## Erinnerungen und Kalender

Erinnerungen an Aufgaben werden als Wecker auf dem Handy gestellt und
erscheinen als Benachrichtigungen, auch ohne Netz. Ihre Aktion
„Erledigt“ erledigt die Aufgabe; bei einer mit offenen Aufgaben darunter
heißt sie „Erledigen …“ und öffnet die Aufgabe. Termine stehen im
Forecast und in der Kalenderansicht (Liste, Tag, Woche); Tippen öffnet
das Blatt des Termins, nur lesbar, mit „In Kalender-App öffnen“.

**Erinnerungen an Termine** (Einstellungen › „Erinnerungen“) sind
anfangs aus und gelten nur für dieses Handy. Eingeschaltet melden sich
die Erinnerungen, die ein Termin in seinem Kalender trägt (VALARM), wie
die von Aufgaben, mit Titel, Uhrzeit und Ort; „Später“ fragt in einer
Stunde wieder, Tippen öffnet den Termin. Gleicht das Handy die Kalender
schon über DAVx5 ab, bleibt der Schalter besser aus, sonst kommt jede
Erinnerung zweimal. Das Blatt des Termins nennt seine Erinnerungen
(„15 Min. vorher“), ob der Schalter an ist oder nicht.

## Sprache

Einstellungen › „Sprache“: „System“, „Deutsch“ oder „English“, nur auf
diesem Handy. „System“ folgt der Sprache des Telefons und der App-Sprache
in Androids Einstellungen (ab Android 13). Ein Wechsel zeichnet sofort
neu; Widgets und Benachrichtigungen folgen beim nächsten Zeichnen.
Fehlermeldungen des Servers bleiben englisch.

## Automatisieren mit Tasker

Zwei Broadcasts lassen andere Apps (etwa Tasker) auf die App wirken; ihr
Paketname ist `de.samudo.app`.

**Scope** (dem Kontext folgen: Nicht stören, WLAN, Ort):

| Feld | Wert |
|---|---|
| Aktion | `de.samudo.app.action.SCOPE` |
| Paket | `de.samudo.app` |
| Extra | `scope`: Titel oder ID eines Scopes; leer = kein Scope; `auto` = dem Zeitplan folgen |

**In die Inbox erfassen** (wie der Ring, auch offline): Aktion
`de.samudo.app.action.RING`, Extra `text`, wahlweise `recorded_at` in
Unix-Millisekunden. Siehe
[Erfassen vom Index-Ring](ring.md#offline-über-tasker).

Über adb ausprobieren:

```sh
adb shell am broadcast -a de.samudo.app.action.SCOPE -p de.samudo.app --es scope Work
adb shell am broadcast -a de.samudo.app.action.RING -p de.samudo.app --es text "'Milch kaufen'"
```

Zurück zum Anmeldebildschirm: `adb shell pm clear de.samudo.app`.
