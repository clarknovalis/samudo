# Desktop widget

samuAgenda is a plugin for the bar of
[DankMaterialShell](https://github.com/AvengeMedia/DankMaterialShell) (a
desktop shell for Wayland compositors such as niri and Hyprland). It shows
the forecast with appointments and lets you complete, flag, clock and
capture. Everything goes through the [command line](cli.md) and its
session; it follows changes live through `samu events`. It lives in a
repository of its own, [samudo-dms](https://github.com/clarknovalis/samudo-dms).

## Installing

```sh
git clone https://github.com/clarknovalis/samudo-dms ~/.config/DankMaterialShell/plugins/samuAgenda
dms ipc call plugins enable samuAgenda
```

Then add `samudo` to the bar in the settings under bar widgets.

## Settings

| Setting | Default | |
|---|---|---|
| Scope | as on this device | or no scope, or a scope from `samu scope list` |
| samu command | `samu` | name or path |
| Profile | as the command line | a [profile](profiles.md) from `samu profile list` |
| Open with | `samu open-uri samu:{id}` | a shell command, `{id}` is replaced; for Emacs e.g. `emacsclient -n -e '(samu-show "{id}")'` |
| Fallback refresh | 15 minutes | without an event, e.g. after midnight |
| Show "later" | off | |
| Clock task in the bar | on | else only the time |

### One widget per profile

"Ein Widget je Profil" › "Widget anlegen" makes a variant for one profile.
It appears under bar › add widget as "samudo - *profile*" and shows that
account, with the scope `samu --profile <profile> scope use` chose there.

## Keys in the popout

| Key | |
|---|---|
| ↑ ↓, `j` `k` | select |
| Enter, double click | open |
| `x`, Space, the circle | complete |
| `f` | flag on/off |
| `c` | start the clock on the task, stop the running one |
| `a`, Tab | capture; Enter creates, Escape clears or goes back |
| `r` | reload |

## From outside

```sh
dms ipc call samuAgenda toggle             # popout open/closed
dms ipc call samuAgenda capture            # popout on the capture tab
dms ipc call samuAgenda add "Milk #Shopping"
dms ipc call samuAgenda clockOut
dms ipc call samuAgenda refresh
```

In niri, for example: `Mod+T { spawn "dms" "ipc" "call" "samuAgenda" "capture"; }`.
