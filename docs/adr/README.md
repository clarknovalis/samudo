# Decision records

Every decision that shapes samudo has a record here: context, decision,
consequences. A comment in the code that cites `ADR NNNN §n` points to
one of them.

- A record is a **living document** ([0133](0133-decision-records-in-the-repository.md)).
  When a decision changes, its record changes, with a dated entry under
  "Changes" that says what was true before. Numbers and section numbers
  stay, so that citations in the code keep pointing at the right place.
- Status is `draft`, `accepted` or `withdrawn`. A draft has a number and
  may change freely.
- Records are written in English.

Records 0001 to 0132 were written in German outside this repository.
They are being translated, the most cited first; until a record is here,
its citations in the code have no target.

| ADR | Status | Decision |
|---|---|---|
| [0004](0004-sync-server-is-the-truth.md) | accepted | The server is the truth; offline clients keep a mirror and an outbox stamped with an HLC; last-writer-wins per field |
| [0015](0015-unknown-values-and-hlc-on-the-wire.md) | accepted | Unknown enum values are read as `unknown` and never written; the HLC is an object `{wall, counter, device}` on the wire |
| [0017](0017-availability-edge-cases.md) | accepted | Availability edge cases: dates from midnight in the account's zone, overdue from the next day, due soon P2D, `unknown` as not active, cycles without a crash |
| [0024](0024-workflows.md) | accepted | Workflows are exclusive tag groups: an LWW register per object, a workflow per project with a default, keys and cycling, a logbook |
| [0027](0027-route-capture.md) | accepted | Quick capture over `POST /capture` with a preview; unknown tags created on the way, never projects; suggestions while typing |
| [0028](0028-android-app.md) | accepted | Android app in Kotlin/Compose over `samu-mobile` (uniffi); mirror plus outbox, sync in Rust, overlay in `samu-core` |
| [0030](0030-reminders.md) | accepted | Reminders as the object `reminder`, due tasks remind by themselves, computed in `samu-core`, one alarm on the phone |
| [0035](0035-scopes.md) | accepted | Scopes: one lens over all views, rule in `samu-core`, chosen per device, due tasks outside counted in the header |
| [0036](0036-managing-objects.md) | accepted | Deleting through `POST /objects/{id}/delete`, planned in `samu-core` and written in one transaction; orphaned tasks count as inbox |
| [0037](0037-settings-second-round.md) | accepted | Moving projects, folders and tags as ordinary changes without cycles; project properties; visible folders from `samu-core`; navigation, tree and inspector |
| [0039](0039-comments-attachments-author.md) | accepted | Comments and attachments as objects; blobs by SHA-256 in samu; the server sets the author `{device, kind}` from the session |
| [0043](0043-scopes-second-round.md) | accepted | Scopes: inbox as a checkbox, filter in the search language, capture defaults, schedule windows, choice over Tasker |
| [0045](0045-forecast-second-round.md) | accepted | Forecast computed in `samu-core` with `GET /forecast`: flagged section, forecast tag, deferred tasks, a day strip; account settings as the object `settings` |
| [0046](0046-clocking.md) | accepted | Clocking over `/changes`: one running clock per account, the newer wins, completing stops it; evaluation in `samu-core` |
| [0047](0047-calendars.md) | accepted | Calendars over ICS and CalDAV fetched by the server, sources encrypted; appointments expanded in `samu-core`, in forecast and calendar view |
| [0054](0054-properties-in-the-interface.md) | accepted | Properties as foldable frontmatter, link templates per key, local commands through `samu-open:`, inheritance in `samu-core`, chip `prop:` |
| [0058](0058-rendered-notes-and-comments.md) | accepted | Notes and comments rendered: Org or Markdown detected per text, a document tree in `samu-api`, `POST /render`, safe by construction |
| [0066](0066-offline-on-the-computer.md) | accepted | A local replica on 127.0.0.1 with mirror and outbox answers the API and serves the web UI; every route is local, online only or relayed |
| [0068](0068-writing-appointments.md) | accepted | Appointments written back over CalDAV, offline through the outbox; series edits in `samu-core`; collisions per field |
| [0072](0072-scope-as-workspace.md) | accepted | A scope carries a layout on the server, first for the inspector; parts not shown are folded and named when they have a value |
| [0073](0073-capture-templates.md) | accepted | Capture templates: a quick-capture line as defaults plus reminders, invoked with `/name`; line before template before scope |
| [0074](0074-export-and-restore.md) | accepted | Export as a directory of stable JSON and files, readable without samu; restore only into an empty account; Org as an output only |
| [0075](0075-opening-references.md) | accepted | Opening references: `/o/<uuid>` and an address for every view in the web UI, `samu:` handlers on desktop, Emacs and Android, `catastr:` as link class `app` |
| [0078](0078-edit-dialog.md) | accepted | The "Edit" dialog changes an existing task in quick-capture syntax, its values as removable chips; `CaptureRequest.task` |
| [0080](0080-report.md) | accepted | Report of done, worked on and time over a period, filtered with the search language, `GET /report`; by default only the person's own work |
| [0081](0081-report-replaces-time.md) | accepted | "Report" replaces "Time"; booking time afterwards with `z:` (and `x:` for completed) in capture and edit |
| [0082](0082-samu-el.md) | accepted | samu.el through the CLI: views like org-agenda, the web's keys, a task's buffer, capture with completion, the clock in the mode line; writing waits, reading runs in the background |
| [0088](0088-review.md) | accepted | Review: due date computed in `samu-core`, "reviewed" as an ordinary change, a review view in the web |
| [0090](0090-perspectives.md) | accepted | Perspectives: saved search, filter, arrangement and display as a server object, evaluated in `samu-core`, in the sidebar |
| [0091](0091-habits.md) | accepted | Habits: a task with the field `habit`, every tap an entry `habit_entry`, streak and rate in `samu-core` |
| [0094](0094-only-in-scope.md) | accepted | A folder, project or tag names its own scope (`own_scope`) and appears only there, whatever the scope's mode |
| [0095](0095-restricted-session.md) | accepted | A session can be restricted at login to granted folders and projects, read-write or read-only; every route has a rule |
| [0097](0097-hardening-a-public-instance.md) | accepted | A public instance: a login brake, limits on attachment size and per account, allowed calendar hosts |
| [0099](0099-registration-and-demo-content.md) | accepted | Registration off, demo-only or open; demo accounts expire; demo content built in the server |
| [0100](0100-managing-and-viewing-calendars.md) | accepted | More calendars from a configured server, importing ICS files, month and list views, a repetition editor for appointments |
| [0101](0101-appointment-reminders.md) | accepted | Reminders for appointments from VALARM, computed in `samu_core::reminders`, `GET /alarms`, a switch per device |
| [0102](0102-bar-and-perspectives-in-the-app.md) | accepted | The app's bottom bar is a configurable list per device; perspectives as lists; a slimmer header; long press opens a context menu |
| [0105](0105-interface-language.md) | accepted | English beside German in web and app, chosen per device; the search language understands both and stores German |
| [0115](0115-table-view.md) | accepted | A table as the third display beside list and board, with choosable columns including properties and sums of estimate and actual time |
| [0116](0116-block-perspectives.md) | accepted | Block perspectives: several perspectives and fixed views in one view, evaluated in `samu-core`, `GET /blocks` |
| [0128](0128-closing-with-open-children.md) | accepted | Completing or dropping with open children asks: close them along, lift them out, or cancel; `POST /objects/{id}/close` |
| [0129](0129-capturing-several-lines.md) | accepted | Capturing several lines: each line a task, indentation makes subtasks, `POST /capture/lines` all or nothing |
| [0133](0133-decision-records-in-the-repository.md) | accepted | Decision records live in this repository, in English, as living documents |
| [0134](0134-one-history-on-github.md) | accepted | One repository with its real history on GitHub; hooks keep private words out; only the maintainer pushes, agents do not |
| [0135](0135-modules-as-the-one-cut.md) | accepted | One table of modules cuts both what is shown and where code lives; shared files know a module only through a registration |
| [0136](0136-removing-a-feature.md) | accepted | A feature leaves in three stages: deprecated, retired, removed |
| [0137](0137-published-container-image.md) | accepted | A container image on ghcr.io for amd64 and arm64: `X.Y.Z`, `X.Y` and `latest` from a release tag, `edge` from `main` after CI |
| [0140](0140-voice-capture.md) | accepted | Voice capture: an optional transcript service at the server, a recorder in the app that can be the digital assistant, recordings handed between apps by intent |
| [0141](0141-checklists.md) | accepted | Checklists: items of a task as objects of their own, typed and pasted fast, turned into subtasks and back |
