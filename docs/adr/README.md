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
| [0015](0015-unknown-values-and-hlc-on-the-wire.md) | accepted | Unknown enum values are read as `unknown` and never written; the HLC is an object `{wall, counter, device}` on the wire |
| [0035](0035-scopes.md) | accepted | Scopes: one lens over all views, rule in `samu-core`, chosen per device, due tasks outside counted in the header |
| [0039](0039-comments-attachments-author.md) | accepted | Comments and attachments as objects; blobs by SHA-256 in samu; the server sets the author `{device, kind}` from the session |
| [0043](0043-scopes-second-round.md) | accepted | Scopes: inbox as a checkbox, filter in the search language, capture defaults, schedule windows, choice over Tasker |
| [0047](0047-calendars.md) | accepted | Calendars over ICS and CalDAV fetched by the server, sources encrypted; appointments expanded in `samu-core`, in forecast and calendar view |
| [0068](0068-writing-appointments.md) | accepted | Appointments written back over CalDAV, offline through the outbox; series edits in `samu-core`; collisions per field |
| [0091](0091-habits.md) | accepted | Habits: a task with the field `habit`, every tap an entry `habit_entry`, streak and rate in `samu-core` |
| [0094](0094-only-in-scope.md) | accepted | A folder, project or tag names its own scope (`own_scope`) and appears only there, whatever the scope's mode |
| [0095](0095-restricted-session.md) | accepted | A session can be restricted at login to granted folders and projects, read-write or read-only; every route has a rule |
| [0099](0099-registration-and-demo-content.md) | accepted | Registration off, demo-only or open; demo accounts expire; demo content built in the server |
| [0133](0133-decision-records-in-the-repository.md) | accepted | Decision records live in this repository, in English, as living documents |
| [0134](0134-one-history-on-github.md) | accepted | One repository with its real history on GitHub; hooks keep private words out; agents push with a token of their own |
| [0135](0135-modules-as-the-one-cut.md) | accepted | One table of modules cuts both what is shown and where code lives; shared files know a module only through a registration |
| [0136](0136-removing-a-feature.md) | accepted | A feature leaves in three stages: deprecated, retired, removed |
