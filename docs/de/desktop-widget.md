# Desktop-Widget

samuAgenda ist ein Plugin für die Leiste von
[DankMaterialShell](https://github.com/AvengeMedia/DankMaterialShell)
(eine Desktop-Shell für Wayland-Compositoren wie niri und Hyprland). Es
zeigt den Forecast mit Terminen und lässt erledigen, markieren, die Uhr
starten und erfassen. Alles läuft über die [Kommandozeile](cli.md) und
ihre Sitzung; Änderungen folgt es live über `samu events`. Es liegt in
einem eigenen Repository,
[samudo-dms](https://github.com/clarknovalis/samudo-dms).

## Installieren

```sh
git clone https://github.com/clarknovalis/samudo-dms ~/.config/DankMaterialShell/plugins/samuAgenda
dms ipc call plugins enable samuAgenda
```

Dann in den Einstellungen unter den Widgets der Leiste `samudo`
hinzufügen.

## Einstellungen

| Einstellung | Vorgabe | |
|---|---|---|
| Scope | wie auf diesem Gerät | oder kein Scope, oder ein Scope aus `samu scope list` |
| samu-Kommando | `samu` | Name oder Pfad |
| Profil | wie die Kommandozeile | ein [Profil](profiles.md) aus `samu profile list` |
| Öffnen mit | `samu open-uri samu:{id}` | ein Shell-Kommando, `{id}` wird ersetzt; für Emacs etwa `emacsclient -n -e '(samu-show "{id}")'` |
| Ersatz-Aktualisierung | 15 Minuten | ohne Ereignis, etwa nach Mitternacht |
| „Später“ zeigen | aus | |
| Aufgabe der Uhr in der Leiste | an | sonst nur die Zeit |

### Ein Widget je Profil

„Ein Widget je Profil“ › „Widget anlegen“ macht eine Variante für ein
Profil. Sie erscheint unter Leiste › Widget hinzufügen als „samudo -
*profil*“ und zeigt dieses Konto, mit dem Scope, den
`samu --profile <profil> scope use` dort gewählt hat.

## Tasten im Popout

| Taste | |
|---|---|
| ↑ ↓, `j` `k` | auswählen |
| Enter, Doppelklick | öffnen |
| `x`, Leertaste, der Kreis | erledigen |
| `f` | Markierung an/aus |
| `c` | Uhr an der Aufgabe starten, die laufende anhalten |
| `a`, Tab | erfassen; Enter legt an, Escape leert oder geht zurück |
| `r` | neu laden |

## Von außen

```sh
dms ipc call samuAgenda toggle             # Popout auf/zu
dms ipc call samuAgenda capture            # Popout auf dem Reiter Erfassen
dms ipc call samuAgenda add "Milch #Einkauf"
dms ipc call samuAgenda clockOut
dms ipc call samuAgenda refresh
```

In niri zum Beispiel:
`Mod+T { spawn "dms" "ipc" "call" "samuAgenda" "capture"; }`.
