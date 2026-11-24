// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

// A date moved to another day in web/lib/zoned.js (ADR 0144 §3): a time
// of day is kept, with the offset of its zone on the new day. Run by
// `make check` with node, no packages; synthetic data only.

const root = new URL("../../", import.meta.url);
const { atTime, onDay, timeOf } = await import(new URL("web/lib/zoned.js", root));

let failed = 0;
const check = (what, got, want) => {
  const g = JSON.stringify(got);
  const w = JSON.stringify(want);
  if (g !== w) {
    failed += 1;
    console.error(`${what}\n  got  ${g}\n  want ${w}`);
  }
};

check("a whole day stays one", onDay({ date: "2026-11-20" }, "2026-11-27"), { date: "2026-11-27" });
check("no date becomes a day", onDay(null, "2026-11-27"), { date: "2026-11-27" });
check("an emptied field clears", onDay({ at: "2026-11-20T09:30:00+01:00[Europe/Berlin]" }, ""), null);
check(
  "a time is kept",
  onDay({ at: "2026-11-20T09:30:00+01:00[Europe/Berlin]" }, "2026-11-27"),
  { at: "2026-11-27T09:30:00+01:00[Europe/Berlin]" },
);
check(
  "into summer time the offset follows the zone",
  onDay({ at: "2026-03-20T14:30:00+01:00[Europe/Berlin]" }, "2026-04-02"),
  { at: "2026-04-02T14:30:00+02:00[Europe/Berlin]" },
);
check(
  "out of summer time, west of UTC",
  onDay({ at: "2026-10-30T08:00:00-04:00[America/New_York]" }, "2026-11-02"),
  { at: "2026-11-02T08:00:00-05:00[America/New_York]" },
);
check(
  "an unknown zone keeps the day",
  onDay({ at: "2026-11-20T09:30:00+01:00[Nowhere/Never]" }, "2026-11-27"),
  { date: "2026-11-27" },
);

// A time field next to the day (ADR 0144 §3, §5).
check("a time on a day", atTime("2026-11-25", "14:30", "Europe/Berlin"), {
  at: "2026-11-25T14:30:00+01:00[Europe/Berlin]",
});
check("a time in summer", atTime("2026-07-01", "09:05", "Europe/Berlin"), {
  at: "2026-07-01T09:05:00+02:00[Europe/Berlin]",
});
check("no time is a whole day", atTime("2026-11-25", "", "Europe/Berlin"), { date: "2026-11-25" });
check("no day is nothing", atTime("", "14:30", "Europe/Berlin"), null);
check("an unknown zone keeps the day", atTime("2026-11-25", "14:30", "Nowhere/Never"), { date: "2026-11-25" });
check("the time of an instant", timeOf({ at: "2026-11-25T09:05:00+01:00[Europe/Berlin]" }), "09:05");
check("a whole day has none", timeOf({ date: "2026-11-25" }), "");
check("nothing has none", timeOf(null), "");

if (failed) {
  console.error(`${failed} failed`);
  process.exit(1);
}
console.log("zoned: ok");
