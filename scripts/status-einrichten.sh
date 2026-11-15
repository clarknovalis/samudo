#!/usr/bin/env bash
#
# Richtet in einem Konto den Ablauf „Status" ein (ADR 0024), über die CLI:
#
#   Status         ausschließend, Vorgabe für Projekte ohne eigenen Ablauf
#   ├─ In Arbeit   aktiv,   Kürzel a
#   ├─ Wartet      on hold, Kürzel w, fragt nach einer Notiz
#   └─ Irgendwann  on hold, Kürzel i
#
# Vorhandene Tags `Waiting` und `Someday` (aus `samu import org`) werden zu
# `Wartet` und `Irgendwann` umbenannt und unter `Status` gehängt; die
# Aufgaben behalten sie. Ein zweiter Lauf ändert nichts.
#
#   scripts/status-einrichten.sh            # gegen das Konto aus der Konfiguration
#   SAMU=/pfad/zu/samu scripts/status-einrichten.sh
set -euo pipefail

SAMU="${SAMU:-samu}"

tags() { "$SAMU" tag list --format json; }
# Die Id eines Tags nach seinem Pfad, leer wenn es ihn nicht gibt.
id_of() { tags | jq -r --arg p "$1" '.[] | select(.path == $p) | .id'; }

gruppe="$(id_of Status)"
if [ -z "$gruppe" ]; then
  "$SAMU" tag add Status --exclusive --default-workflow >/dev/null
  gruppe="$(id_of Status)"
  echo "Status angelegt"
fi

# zustand <Titel> <alter Titel> <Kürzel> <Position> <on_hold|active> [--ask-note]
zustand() {
  local titel="$1" alt="$2" key="$3" pos="$4" status="$5" frage="${6:-}"
  local id
  id="$(id_of "Status/$titel")"
  if [ -z "$id" ] && [ -n "$alt" ]; then
    id="$(id_of "$alt")"
    if [ -n "$id" ]; then
      "$SAMU" set "$id" title "$titel"
      "$SAMU" set "$id" parent "\"$gruppe\""
      echo "$alt → Status/$titel"
    fi
  fi
  if [ -z "$id" ]; then
    local args=(tag add "$titel" --parent Status)
    [ "$status" = on_hold ] && args+=(--on-hold)
    "$SAMU" "${args[@]}" >/dev/null
    id="$(id_of "Status/$titel")"
    echo "Status/$titel angelegt"
  fi
  "$SAMU" set "$id" key "$key"
  "$SAMU" set "$id" position "$pos"
  "$SAMU" set "$id" status "$status"
  if [ "$frage" = --ask-note ]; then
    "$SAMU" set "$id" ask_note true
  fi
}

zustand "In Arbeit"  ""        a 0 active
zustand "Wartet"     Waiting   w 1 on_hold --ask-note
zustand "Irgendwann" Someday   i 2 on_hold

"$SAMU" tag list
