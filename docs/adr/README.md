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
| [0133](0133-decision-records-in-the-repository.md) | accepted | Decision records live in this repository, in English, as living documents |
| [0134](0134-one-history-on-github.md) | accepted | One repository with its real history on GitHub; hooks keep private words out; agents push with a token of their own |
| [0135](0135-modules-as-the-one-cut.md) | accepted | One table of modules cuts both what is shown and where code lives; shared files know a module only through a registration |
| [0136](0136-removing-a-feature.md) | accepted | A feature leaves in three stages: deprecated, retired, removed |
