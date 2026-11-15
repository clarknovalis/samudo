// Addresses the settings show but never call: the ready link templates
// (ADR 0047 §9) and the examples in empty fields. Kept here, apart, so the
// test that the UI calls only relative paths can leave this one file out.

// Each opens the day view on the date. Checked before they came in
// (09.11.2026):
// - Google answers the path with 200 and opens the day.
// - Nextcloud: the calendar app's router takes `/:view/:firstDay` with
//   `firstDay` as `YYYY-MM-DD` (nextcloud/calendar, src/router.js);
//   `index.php/` works with and without pretty URLs.
// Outlook on the web is left out until its day path is checked.
export const LINK_TEMPLATES = [
  ["Google", () => "https://calendar.google.com/calendar/r/day/{y}/{m}/{d}"],
  ["Nextcloud", (calendar) => `https://${calendar?.host ?? "cloud.example.org"}/index.php/apps/calendar/timeGridDay/{date}`],
];

export const EXAMPLES = {
  ics: "https://calendar.google.com/calendar/ical/…/basic.ics",
  caldav: "https://dav.example.org/calendars/me/work/",
  server: "https://caldav.example.org",
  link: "https://…/{date}",
  // A property's template (ADR 0054 §3).
  property: "https://jira.example.org/browse/{value}",
};
