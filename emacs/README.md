# samu.el

samudo aus Emacs, über die CLI `samu` mit `--format json`. Beschlossen in
ADR 0082.

**Benutzung:** [`docs/emacs.md`](../docs/emacs.md) (englisch): Laden,
Einstellungen, Ansichten, Tasten, Habits, Erfassen, Org-Links, Live.

## Tests

`make emacs-check` (Teil von `make check`) kompiliert ohne Warnungen und
lässt `test-samu.el` laufen, ohne CLI und ohne Server.
