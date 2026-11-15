# Web-UI

Oberfläche über die HTTP-API und nichts sonst (ADR 0025), drei Spalten
(ADR 0020), Gestalt nach der Designstudie (ADR 0026). Ausgeliefert vom
Server unter `/`.

**Benutzung:** [`docs/web.md`](../docs/web.md) (englisch), die Suchsprache in
[`docs/search.md`](../docs/search.md). Hier steht nur, wie sie gebaut ist.

## Tasten

Jede Aktion steht mit ID, Vorgabe und Ort in `ACTIONS` in `lib/keymap.js`.
Eigene Belegungen liegen in `settings.keys` (ADR 0079). Tasten, Hilfe und
Kontextmenü lesen alle aus dieser Tabelle und können deshalb nicht
voneinander abweichen.

## Aussehen

Farben nur als Tokens oben in `style.css` (ADR 0026 §1): hell aus Entwurf
A, dunkel mit Bs Farben; das dunkle Schema greift nach dem System oder
nach der Wahl im Kontomenü (`data-theme` auf `<html>`, gemerkt je Browser).
Schrift Inter, wo installiert, sonst die des Systems; die Seite lädt nichts
von außen.

## Was rechnet

Nichts hier. Verfügbarkeit, Gründe, Zustand, nächster Zustand und die
Zusammenfassung einer Gruppe kommen aus `GET /available` (ADR 0025 §3).
Welche Projekte feststecken, kommt aus `GET /available/projects`. Die
Oberfläche zählt und gruppiert nur, was der Server geschickt hat.
Die Auswertung der Zeit kommt aus `GET /clocks/report`. Gezählt wird
hier nur die mitlaufende Zeit der laufenden Uhr und im Inspektor die
Summe der Clocks einer Aufgabe: Abstände zwischen Zeitpunkten, die der
Server geschickt hat.

## Kein Framework, keine Build-Kette

ES-Module, nichts zu bauen. `app.js` verdrahtet, `lib/` enthält die Teile:
`api.js` (der einzige Weg zum Server), `state.js` (aller Zustand), `dom.js`
(`el()` baut aus Text, nie aus HTML), `sidebar.js`, `list.js`,
`detail.js`, `comments.js` (Kommentare und Anhänge, ADR 0039), `capture.js`, `palette.js` (Refile und Ein-/Ausrücken, der
einzige Weg zum Umhängen), `keys.js`, `live.js` (`/events`, nur bei
sichtbarem Tab), `account.js`, `words.js` (Gründe und Termine in Worten), `clock.js`
(Statuszeile, Abschnitt und Ansicht „Zeit“, ADR 0046), `calendar.js`
(Ansicht „Kalender“ und der Termin im Inspektor, ADR 0047 §11).

Der Server trägt die Dateien im Binary (`crates/samu-server/src/web.rs`).
Eine neue Datei muss dort in `FILES` stehen; die Tests dort prüfen, dass
jedes Element, jedes Modul und jede Route, die ein Skript nennt, existiert.

## Entwickeln

```sh
cargo build -p samu-server
target/debug/samu-server --data /tmp/samu serve --listen 127.0.0.1:8492
```

Dann `http://localhost:8492/`. Der Cookie trägt `Secure`; Browser nehmen
ihn auf `localhost` trotzdem an, auf einer anderen `http://`-Adresse nicht.
