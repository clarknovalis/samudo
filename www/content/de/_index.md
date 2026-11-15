---
title: samudo
layout: hextra-home
---

<div class="hx:mt-6 hx:mb-6">
{{< hextra/hero-badge link="#open-source" >}}
  Open Source · AGPL-3.0 · noch kein Release
{{< /hextra/hero-badge >}}
</div>

<div class="hx:mt-6 hx:mb-6">
{{< hextra/hero-headline >}}
  Aufgaben, selbst gehostet.
{{< /hextra/hero-headline >}}
</div>

<div class="hx:mb-12">
{{< hextra/hero-subtitle >}}
  Das Beste aus OmniFocus und org-mode, mit einem Touch Jira.&nbsp;<br class="hx:sm:block hx:hidden" />Ein Server, viele Clients: Web, Android, Kommandozeile, Emacs.
{{< /hextra/hero-subtitle >}}
</div>

<div class="hx:mb-16 samudo-buttons">
{{< hextra/hero-button text="Demo ausprobieren" link="https://demo.samudo.de/" >}}
{{< hextra/hero-button text="Dokumentation" link="https://docs.samudo.de/" style="background: transparent; color: inherit; border: 1px solid currentColor;" >}}
</div>

{{< hextra/feature-grid >}}
  {{< hextra/feature-card title="Verfügbar statt nur fällig" icon="check-circle"
      subtitle="Zurückstellen und Fälligkeit sind getrennt. Was gerade verfügbar ist, wird berechnet, und jede Ansicht sagt, warum etwas nicht verfügbar ist." >}}
  {{< hextra/feature-card title="Projekte, Abläufe, Board" icon="view-boards"
      subtitle="Sequenzielle und parallele Projekte, Aktionsgruppen, Abläufe mit Zuständen und ein Board je Projekt." >}}
  {{< hextra/feature-card title="Erfassen in einer Zeile" icon="lightning-bolt"
      subtitle="Fliesenleger anrufen #Bad @telefon d:fr ~15m – Projekt, Tag, Termin und Dauer in einem Satz." >}}
  {{< hextra/feature-card title="Perspektiven, Forecast, Kalender" icon="calendar"
      subtitle="Gespeicherte Suchen, Reviews, Scopes und ein Kalender mit deinen Terminen über CalDAV." >}}
  {{< hextra/feature-card title="Zeit, Gewohnheiten, Erinnerungen" icon="clock"
      subtitle="Zeiterfassung mit Bericht, Gewohnheiten mit Serie, Erinnerungen, Kommentare und Anhänge." >}}
  {{< hextra/feature-card title="Deine Daten" icon="database"
      subtitle="Ein Server, den du selbst betreibst. Export als JSON-Dateien und als Org-Gliederung, lesbar ohne samudo." >}}
{{< /hextra/feature-grid >}}

<div class="samudo-section">

## So sieht es aus

Die Web-Oberfläche mit dem Demo-Konto `demo-de`. Die Aufgaben sind eine
Huldigung an Terry Pratchett und Douglas Adams.

<div class="samudo-screens">

{{< gallery cols="2" gap="1.5rem" >}}
  {{< gallery-item src="screenshots/de/inbox.png" caption="Die Inbox: alles, was ohne Projekt erfasst wurde" >}}
  {{< gallery-item src="screenshots/de/project-detail.png" caption="Ein Projekt mit Aktionsgruppe, rechts die Details einer Aufgabe" >}}
  {{< gallery-item src="screenshots/de/board.png" caption="Dasselbe Projekt als Board, eine Spalte je Zustand des Ablaufs" >}}
  {{< gallery-item src="screenshots/de/forecast.png" caption="Forecast: überfällig, heute, markiert, mit Terminen aus dem Kalender" >}}
  {{< gallery-item src="screenshots/de/calendar.png" caption="Der Kalender: geplante Aufgaben und Termine über CalDAV" >}}
  {{< gallery-item src="screenshots/de/perspective.png" caption="Eine Perspektive: eine gespeicherte Suche in der Suchsprache" >}}
  {{< gallery-item src="screenshots/de/habits.png" caption="Gewohnheiten mit Serie und Fortschritt" >}}
  {{< gallery-item src="screenshots/de/report.png" caption="Der Bericht: erledigt, bearbeitet, Zeit" >}}
  {{< gallery-item src="screenshots/de/capture.png" caption="Schnellerfassung: eine Zeile, die Chips zeigen, was verstanden wurde" >}}
  {{< gallery-item src="screenshots/de/dark.png" caption="Alle Projekte im Dunkelmodus" >}}
{{< /gallery >}}

</div>

Und die Android-App, die offline weiterarbeitet und synchronisiert, sobald
sie wieder Netz hat:

<div class="samudo-phones">

{{< gallery cols="4" gap="1rem" >}}
  {{< gallery-item src="screenshots/de/app-inbox.png" alt="Inbox" >}}
  {{< gallery-item src="screenshots/de/app-forecast.png" alt="Forecast" >}}
  {{< gallery-item src="screenshots/de/app-project.png" alt="Ein Projekt" >}}
  {{< gallery-item src="screenshots/de/app-capture.png" alt="Erfassen" >}}
{{< /gallery >}}

</div>

## Oberflächen

Alle sprechen mit demselben Server über dieselbe API; die Kommandozeile
deckt sie vollständig ab.

- [Web-Oberfläche](https://docs.samudo.de/clients/web/) – im Browser, mit Tastaturkürzeln
- [Android-App](https://docs.samudo.de/clients/android/) – offline-fähig, mit Widgets
- [Kommandozeile](https://docs.samudo.de/clients/cli/) – `samu`, für Skripte mit JSON-Ausgabe
- [Emacs](https://docs.samudo.de/clients/emacs/) – Ansichten wie org-agenda, Erfassen, die laufende Uhr in der Modeline
- [Desktop-Widget](https://docs.samudo.de/clients/desktop-widget/) – der Forecast in der Leiste von DankMaterialShell

## Ausprobieren

In der [öffentlichen Demo](https://demo.samudo.de/) mit dem Konto `demo-de`
(oder `demo-en`) und dem Passwort `demo4samudo` anmelden. Alle Besucher
teilen sich diese Daten; sie fangen jede Stunde frisch an. Wer in Ruhe
probieren will, legt dort ein eigenes Demo-Konto an; es wird sieben Tage
nach der letzten Benutzung gelöscht. Die App nimmt als Server
`https://demo.samudo.de`.

## Selbst hosten

samudo ist ein Programm, `samu-server`, mit SQLite-Dateien in einem
Verzeichnis; ein halber Prozessorkern und 256 MB reichen. Die Anleitung
mit Docker Compose steht unter
[Betrieb › Selbst hosten](https://docs.samudo.de/running/self-hosting/).
Gedacht ist es für das eigene Heimnetz oder ein VPN, nicht als offener
Dienst im Internet.

## Open Source

samudo steht unter der [GNU Affero General Public License v3.0 oder
später](https://www.gnu.org/licenses/agpl-3.0.html); die Datentypen der
API zusätzlich unter MIT oder Apache-2.0, damit Clients sie ohne Copyleft
einbinden können. Der Quellcode wird auf GitHub veröffentlicht; der Link
folgt hier, sobald es so weit ist.

## Wie samudo entsteht

samudo ist das Projekt einer einzelnen Person, die es täglich benutzt. Sie
entwirft und entscheidet: was samudo können soll, wie es sich anfühlt, und
jede Architekturentscheidung, festgehalten als eigenes Dokument mit Kontext
und Begründung. Den Code schreibt großteils ein KI-Agent (Claude Code),
in kleinen Schritten gegen diese Entscheidungen. Was er schreibt, muss
durch Tests, Linter und Formatprüfung, bevor es gilt, und wird von Hand
ausprobiert, bevor es ausgerollt wird. Das soll weder verschwiegen noch
herausgestellt werden: Es ist schlicht die Art, wie dieses Projekt gebaut
wird.

</div>
