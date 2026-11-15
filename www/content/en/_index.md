---
title: samudo
layout: hextra-home
---

<div class="hx:mt-6 hx:mb-6">
{{< hextra/hero-badge link="#open-source" >}}
  Open source · AGPL-3.0 · no release yet
{{< /hextra/hero-badge >}}
</div>

<div class="hx:mt-6 hx:mb-6">
{{< hextra/hero-headline >}}
  Tasks, self-hosted.
{{< /hextra/hero-headline >}}
</div>

<div class="hx:mb-12">
{{< hextra/hero-subtitle >}}
  The best of OmniFocus and org-mode, with a touch of Jira.&nbsp;<br class="hx:sm:block hx:hidden" />One server, many clients: web, Android, command line, Emacs.
{{< /hextra/hero-subtitle >}}
</div>

<div class="hx:mb-16 samudo-buttons">
{{< hextra/hero-button text="Try the demo" link="https://demo.samudo.de/" >}}
{{< hextra/hero-button text="Documentation" link="https://docs.samudo.de/en/" style="background: transparent; color: inherit; border: 1px solid currentColor;" >}}
</div>

{{< hextra/feature-grid >}}
  {{< hextra/feature-card title="Available, not just due" icon="check-circle"
      subtitle="Defer and due dates are kept apart. What is available right now is computed, and every view says why a task is not." >}}
  {{< hextra/feature-card title="Projects, workflows, a board" icon="view-boards"
      subtitle="Sequential and parallel projects, action groups, workflows with states and a board per project." >}}
  {{< hextra/feature-card title="Capture in one line" icon="lightning-bolt"
      subtitle="Call the tiler #Bathroom @phone d:fri ~15m – project, tag, date and duration in one sentence." >}}
  {{< hextra/feature-card title="Perspectives, forecast, calendar" icon="calendar"
      subtitle="Saved searches, reviews, scopes and a calendar with your appointments over CalDAV." >}}
  {{< hextra/feature-card title="Time, habits, reminders" icon="clock"
      subtitle="Clocking with a report, habits with streaks, reminders, comments and attachments." >}}
  {{< hextra/feature-card title="Your data" icon="database"
      subtitle="A server you run yourself. Export to plain JSON files and an Org outline, readable without samudo." >}}
{{< /hextra/feature-grid >}}

<div class="samudo-section">

## What it looks like

The web UI with the demo account `demo-en`. The tasks are a tribute to
Terry Pratchett and Douglas Adams.

<div class="samudo-screens">

{{< gallery cols="2" gap="1.5rem" >}}
  {{< gallery-item src="screenshots/en/inbox.png" caption="The inbox: everything captured without a project" >}}
  {{< gallery-item src="screenshots/en/project-detail.png" caption="A project with an action group, and a task's details on the right" >}}
  {{< gallery-item src="screenshots/en/board.png" caption="The same project as a board, one column per workflow state" >}}
  {{< gallery-item src="screenshots/en/forecast.png" caption="Forecast: overdue, today, flagged, with appointments from the calendar" >}}
  {{< gallery-item src="screenshots/en/calendar.png" caption="The calendar: planned tasks and appointments over CalDAV" >}}
  {{< gallery-item src="screenshots/en/perspective.png" caption="A perspective: a saved search in the search language" >}}
  {{< gallery-item src="screenshots/en/habits.png" caption="Habits with streak and progress" >}}
  {{< gallery-item src="screenshots/en/report.png" caption="The report: done, worked on, time" >}}
  {{< gallery-item src="screenshots/en/capture.png" caption="Quick capture: one line, the chips show what it understood" >}}
  {{< gallery-item src="screenshots/en/dark.png" caption="All projects, in dark mode" >}}
{{< /gallery >}}

</div>

And the Android app, which keeps working offline and syncs as soon as it
is back online:

<div class="samudo-phones">

{{< gallery cols="4" gap="1rem" >}}
  {{< gallery-item src="screenshots/en/app-inbox.png" alt="Inbox" >}}
  {{< gallery-item src="screenshots/en/app-forecast.png" alt="Forecast" >}}
  {{< gallery-item src="screenshots/en/app-project.png" alt="A project" >}}
  {{< gallery-item src="screenshots/en/app-capture.png" alt="Capture" >}}
{{< /gallery >}}

</div>

## Clients

All of them talk to the same server through the same API; the command
line covers all of it.

- [Web UI](https://docs.samudo.de/en/clients/web/) – in the browser, with keyboard shortcuts
- [Android app](https://docs.samudo.de/en/clients/android/) – works offline, with widgets
- [Command line](https://docs.samudo.de/en/clients/cli/) – `samu`, for scripts with JSON output
- [Emacs](https://docs.samudo.de/en/clients/emacs/) – views in the style of org-agenda, capture, the running clock in the mode line
- [Desktop widget](https://docs.samudo.de/en/clients/desktop-widget/) – the forecast in the DankMaterialShell bar

## Try it

Sign in to the [public demo](https://demo.samudo.de/) as `demo-en` (or
`demo-de`) with the password `demo4samudo`. Every visitor shares this
data; it starts fresh every hour. To try things in peace, make a demo
account of your own there; it is deleted seven days after its last use.
In the app, the server is `https://demo.samudo.de`.

## Self-hosting

samudo is one program, `samu-server`, with SQLite files in a directory;
half a CPU and 256 MB are plenty. The guide with Docker Compose is at
[Running › Self-hosting](https://docs.samudo.de/en/running/self-hosting/).
It is meant for your home network or a VPN, not as an open service on the
internet.

## Open source

samudo is licensed under the [GNU Affero General Public License v3.0 or
later](https://www.gnu.org/licenses/agpl-3.0.html); the wire types of the
API also under MIT or Apache-2.0, so clients can embed them without
copyleft. The source code will be published on GitHub; the link will be
here once it is.

## How samudo is made

samudo is the project of one person who uses it every day. They design and
decide: what samudo should do, how it should feel, and every architectural
decision, each recorded as a document of its own with context and reasons.
Most of the code is written by an AI agent (Claude Code), in small steps
against those decisions. What it writes has to pass tests, linters and
format checks before it counts, and is tried by hand before it is rolled
out. This is neither hidden nor a selling point: it is simply how this
project is built.

</div>
