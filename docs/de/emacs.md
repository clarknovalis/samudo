# Emacs

`emacs/samu.el` bringt samudo nach Emacs: Ansichten im Stil von
org-agenda, Aktionen am Punkt, ein Puffer je Aufgabe, Schnellerfassung, die
laufende Uhr in der Mode-Line und der Org-Linktyp `samu:`. Alles läuft über
die [Kommandozeile](cli.md) mit `--format json`, also sind Konfiguration,
Sitzung und Server die von `samu`, ein lokales [Profil](profiles.md)
eingeschlossen. Emacs schreibt als die Person, die dort angemeldet ist.

## Laden

```elisp
(load "/pfad/zu/samu/emacs/samu.el")
(global-set-key (kbd "C-c s") #'samu)          ; Forecast
(global-set-key (kbd "C-c S") #'samu-capture)  ; Schnellerfassung
(samu-clock-mode-line-mode 1)                  ; laufende Uhr
```

Optionen (`M-x customize-group RET samu`):

| Option | |
|---|---|
| `samu-executable` | die Kommandozeile, vorgegeben `samu` |
| `samu-profile` | wie `--profile`; `nil` = das Standardprofil der Kommandozeile |
| `samu-server`, `samu-account` | wie `--server` / `--account`; `nil` = wie die Kommandozeile |
| `samu-scope` | `nil` = wie die Kommandozeile, `none` = kein Scope, sonst ein Titel; `S` setzt ihn für diese Emacs-Sitzung |
| `samu-link-target` | `emacs` (Vorgabe) oder `browser`; `C-u` beim Folgen nimmt das andere |
| `samu-web-url` | die Web-Oberfläche, für den Browser und den Export |
| `samu-capture-link` | Erfassen aus einer Datei vermerkt die Stelle |
| `samu-live` | offene Puffer und die Uhr folgen Änderungen (vorgegeben an); `samu-toggle-live` |
| `samu-clock-interval` | Sekunden zwischen zwei Aktualisierungen der Uhr |

`samu-switch-profile` und `samu-switch-account` wechseln Profil oder Konto
für diese Emacs-Sitzung.

## Ansichten

| Kommando | |
|---|---|
| `samu`, `samu-forecast` | Forecast: überfällig, heute, markiert, die nächsten Tage, später |
| `samu-inbox` | Aufgaben ohne Projekt |
| `samu-available` | verfügbar, nach Projekt |
| `samu-today` | geplant für heute oder früher |
| `samu-flagged` | markiert |
| `samu-project` | ein Projekt |
| `samu-tag` | ein Tag mit allem darunter |
| `samu-search` | Wörter in Titel, Notiz, ID |
| `samu-projects` | alle Projekte; `RET` öffnet eins |
| `samu-review` | Projekte mit fälligem Review, die ältesten zuerst |
| `samu-show` | eine Aufgabe in einem eigenen Puffer |
| `samu-habit-show` | eine Gewohnheit: Serie, Quote, die letzten 12 Wochen, die Einträge |
| `samu-clock-report` | die erfasste Zeit |

Unter Evil starten die Puffer im Emacs-State.

## Tasten

Die Tasten sind die der [Web-Oberfläche](web.md#tasten), die Navigation
die von Emacs. Mit `M` markierte Aufgaben nehmen
`x f m t s X DEL , : C-c C-s C-c C-d e` gemeinsam.

| Taste | |
|---|---|
| `n` `p`, `j` `k` | nächste / vorige Aufgabe |
| `RET` / `TAB` | Aufgabe zeigen / im anderen Fenster |
| `x` | erledigen / wieder öffnen; mit offenen Aufgaben darunter fragt es: `m` mit erledigen, `h` herausholen, `a` abbrechen |
| `s` / `t` | nächster Zustand / Zustand per Taste |
| `f` | Markierung an/aus |
| `c`, `I` / `O` | Uhr umschalten; ein / aus |
| `m`, `C-c C-w` | verschieben: Inbox oder Projekt, mit `C-u` unter eine Aufgabe |
| `e` | mit Zeichen bearbeiten: `@tag #projekt !A d:fr ~30m every:1w // notiz /vorlage z:30m` |
| `,` | Priorität A/B/C, Leertaste: keine |
| `:` | Tags; fehlende werden angelegt |
| `C-c C-s` / `C-c C-d` | geplant / fällig, `C-u` leert |
| `a` / `A` | erfassen / als Unteraufgabe |
| `C` | mit Vorlage erfassen: danach die Taste der Vorlage, wie in org-capture |
| `/` | suchen |
| `X` / `DEL` | verwerfen / löschen, beide fragen vorher; verwerfen fragt nach offenen Aufgaben darunter wie `x` |
| `w` | den `samu:`-Link kopieren |
| `M` `u` `U` `B` | markieren, Markierung weg, alle weg, Aktion auf die markierten |
| `S` | Scope |
| `R` | Projekt als geprüft markieren |
| `h` | eine [Gewohnheit](#gewohnheiten), gefolgt von einer Taste |
| `g` `q` `?` | neu laden, beenden, Hilfe |

Im Puffer einer Aufgabe außerdem:

| Taste | |
|---|---|
| `T` | Titel |
| `N` | Notiz bearbeiten (Org oder Markdown); `C-c C-c` speichert, `C-c C-k` verwirft |
| `C` | einen Kommentar schreiben, ebenso |
| `+` | eine Datei anhängen |
| `RET` | auf einem Kind: es zeigen; auf einem Anhang: ihn öffnen; auf einer Property: `samu open` |
| `o` | eine Property öffnen |
| `l` | Logbuch zeigen/verbergen |

Aktionstasten wirken auf die Unteraufgabe am Punkt, sonst auf die Aufgabe
des Puffers.

## Gewohnheiten

Eine Zeile einer [Gewohnheit](habits.md) zeigt vor dem Titel, wie es um
den laufenden Zeitraum steht: ✓, sobald das Ziel erreicht ist, ✗ für heute
nicht getan, ‖ für eine Pause, ○ für noch nicht. Mit einer Einheit oder
einem Ziel über eins folgt die Menge, etwa `5/8 Glas` oder `2/3 Woche`, und
dann die Serie, etwa `🔥4` (`samu-habit-streak-mark`). Eine Gewohnheit zum
Vermeiden zeigt `frei seit n T`. Die Zahlen kommen aus `samu habit list`,
einmal bei jedem Füllen eines Puffers, durch denselben Scope wie die
Ansicht.

| Taste | |
|---|---|
| `h h`, `h d` | heute getan: +1. `C-u 3 h h` trägt 3 ein, `C-u h h` fragt nach einem früheren Tag |
| `h f` | heute nicht getan; bei einer Gewohnheit zum Vermeiden ein Rückfall. `C-u` fragt nach dem Tag |
| `h p` | Pause von einem Tag bis zu einem Tag; die Serie reißt nicht |
| `h u` | den zuletzt gemachten Eintrag löschen, nach Rückfrage |
| `h s` | der Puffer der Gewohnheit |

Eintragen erledigt die Gewohnheit nie; das tut `x`, und es beendet sie.
Mit markierten Aufgaben tragen `h h`, `h f` und `h p` für jede davon ein.

Der Aufgabenpuffer einer Gewohnheit zeigt ihr Ziel, heute, die Serie und
die Quoten. Der Puffer der Gewohnheit (`h s`, `samu-habit-show`) zeigt
dazu die letzten 12 Wochen als Kalender, `#` getan, `+` teilweise, `x`
nicht getan, `=` Pause, `.` nichts, und die Einträge dieser Zeit. `DEL`
auf einem Eintrag löscht ihn, `RET` öffnet die Aufgabe, `g` lädt neu.

## Erfassen

`samu-capture` liest eine Zeile der [Schnellerfassung](quick-capture.md).
`TAB` ergänzt `#projekt`, `@tag` und `/vorlage`. Eine aktive Region wird
die Notiz. Aus einer Datei heraus kommt ein Org-Link auf die Zeile dazu
(`samu-capture-link`).

`samu-capture-template` (in den Ansichten `C`, sonst `C-u C-u a`) arbeitet
wie `org-capture`: Ein kleines Fenster listet die Vorlagen mit ihren
Tasten, eine Taste wählt eine, und die Zeile beginnt mit ihrem `/name`.
Eine Taste setzt man mit `samu template key <name> <taste>` oder unter
„Vorlagen“ in den Einstellungen des Webs.

## Org-Links

`samu:<id>` ist ein Org-Linktyp, sobald `ol` geladen ist.

- **Folgen** öffnet eine Aufgabe oder ein Projekt in einem Aufgabenpuffer,
  einen Tag oder Ordner als Ansicht; mit `samu-link-target` = `browser`
  oder `C-u` in der Web-Oberfläche.
- **`org-store-link`** in einem samu-Puffer speichert die Aufgabe am Punkt
  mit ihrem Titel.
- **`C-c C-l samu: RET`** ergänzt über die offenen Aufgaben.
- **Export** ergibt `<samu-web-url>/o/<id>`.

## Live

Solange ein samu-Puffer offen ist oder die Uhr in der Mode-Line steht,
läuft ein Prozess `samu events`. Jede Änderung, auch aus dem Web, der App
oder der Kommandozeile, frischt die offenen Puffer und die Uhr auf, nach
einer Sekunde Ruhe und nicht, während der Minibuffer liest. Punkt,
Markierungen und die erste Zeile jedes Fensters bleiben, wo sie waren.
Endet der Prozess, verbindet er sich nach 5 Sekunden neu, dann mit
verdoppelten Pausen bis zu einer Minute.
