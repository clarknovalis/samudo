# Erfassen vom Index-Ring

Eine Geste am Ring Pebble Index nimmt einen Satz auf; samudo legt ihn als
Aufgabe in der Inbox ab. Es gibt zwei Wege hinein. Der übliche ist ein
Webhook von der Pebble-App an den Server. Der andere geht über Tasker und
die samudo-App auf demselben Handy und funktioniert offline.

## Was in der Inbox landet

Beide Wege legen dieselbe Aufgabe an:

- **Titel:** die erste Zeile des Transkripts, an einer Wortgrenze auf
  höchstens 80 Zeichen gekürzt (`…` markiert den Schnitt). Ohne
  Transkript: `Aufnahme vom Ring, <Datum> <Uhrzeit>` in der Zeitzone des
  Kontos, damit nichts still verloren geht.
- **Notiz:** das ganze Transkript.
- **Angelegt:** die Zeit der Aufnahme, nicht der Zustellung.
- **Properties:** `source=index-ring`; über Tasker zusätzlich `via=tasker`.
- **Keine Schnellerfassung:** Wörter wie `morgen` oder `#tag` bleiben Text.
  Gesprochene Sprache trifft die Kürzel selten, und ein falsch verstandenes
  Datum wäre schlimmer als keins.
- Der Ton wird nicht aufbewahrt; er bleibt in der Pebble-App.

## Webhook von der Pebble-App

Der Server nimmt Zustellungen unter `POST /hooks/<name>` im Format
`index-v1` der Pebble-App an, signiert mit HMAC-SHA256. Eine Anfrage ohne
gültige Signatur, oder älter oder neuer als fünf Minuten, bekommt `401`
und legt nichts an.

1. Einen Hook anlegen. Sein Geheimnis wird **einmal** ausgegeben:

   ```sh
   samu hook add ring
   ```

2. In der Pebble-App für die gewünschte Geste (zum Beispiel
   *Double click & hold*): URL `https://<dein server>/hooks/ring`, Modus
   *Both* oder *Transcription only*, *Sign requests* an und das Geheimnis
   aus Schritt 1.
3. *Send test event*: Der Server antwortet `200` und legt nichts an.
4. Etwas aufnehmen und danach suchen:
   `samu list --search <ein gesprochenes Wort>`.

Die Pebble-App schickt mit jeder Anfrage eine Zustell-ID. Dieselbe ID
noch einmal findet die Aufgabe, die sie schon angelegt hat, statt eine
zweite anzulegen; Wiederholungen schaden also nicht.

Weitere Kommandos:

```sh
samu hook list                  # die Hooks dieses Kontos
samu hook secret ring           # ein neues Geheimnis; das alte gilt sofort nicht mehr
samu hook remove ring           # die angelegten Aufgaben bleiben
samu hook send ring --secret "$SECRET" --text "Milch kaufen"   # ohne Ring ausprobieren
samu hook send ring --secret "$SECRET" --test                  # ein Testereignis
```

## Offline über Tasker

Der Server ist oft nur über ein VPN oder das Heimnetz erreichbar. Ist das
Handy außerhalb von beidem, scheitert die Zustellung der Pebble-App.
Tasker kann den Text stattdessen an die samudo-App geben. Die App legt die
Aufgabe sofort in ihrem Ausgang ab und schickt sie mit dem nächsten Sync.

In Tasker der Aufgabe für die Geste eine Aktion **Send Intent** hinzufügen
(siehe auch [Android-App](android.md#automatisieren-mit-tasker)):

| Feld    | Wert                                           |
|---------|------------------------------------------------|
| Action  | `de.samudo.app.action.RING`                    |
| Package | `de.samudo.app`                                |
| Target  | Broadcast Receiver                             |
| Extra   | `text:<Variable mit dem Transkript>`           |
| Extra   | `recorded_at:<Unix-Millisekunden>` (wahlweise) |

Ohne `recorded_at` gilt die Zeit der Ankunft. Ist in der App niemand
angemeldet oder lässt sich die Aufgabe nicht ablegen, zeigt eine
Benachrichtigung den Text.

Ohne Tasker ausprobieren:

```sh
adb shell am broadcast -a de.samudo.app.action.RING -p de.samudo.app \
    --es text "'Milch kaufen'"
```

**Doppelte:** Dieser Weg weiß nichts von der Zustell-ID des Webhooks.
Kommt dieselbe Aufnahme auf beiden Wegen an, steht sie zweimal in der
Inbox; die von Tasker trägt `via=tasker`.

**Wer senden darf:** Jede App auf dem Handy kann diesen Broadcast senden,
weil Tasker keine eigene Berechtigung halten kann. Er kann nur zur Inbox
hinzufügen, nie etwas lesen oder ändern.
