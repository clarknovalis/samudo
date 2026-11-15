//! The little of CalDAV (RFC 4791) a calendar collection needs to be read
//! (ADR 0047 §7): `PROPFIND` for its ctag or sync-token, to tell whether
//! anything changed, and `REPORT calendar-query` over the window the server
//! keeps. And finding the calendars of a user (ADR 0047 §8, RFC 6764):
//! `.well-known/caldav`, the principal, its calendar home, the collections
//! in there. `sync-collection` comes later. Writing (ADR 0068): the
//! privileges of a collection, and each resource with its href and ETag.
//!
//! Elements are matched by their local name: servers write the DAV,
//! CalDAV and CalendarServer namespaces under whatever prefixes they like.

use jiff::civil::Date;
use quick_xml::Reader;
use quick_xml::events::Event;

/// Asks for what changes whenever the collection does, and what the user
/// may do there (ADR 0068 §1).
pub(crate) const PROPFIND: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:cs="http://calendarserver.org/ns/">
  <d:prop><cs:getctag/><d:sync-token/><d:current-user-privilege-set/></d:prop>
</d:propfind>"#;

/// Every VEVENT with an occurrence from `first` to `last` (included), with
/// its iCalendar text. A repetition counts when one of its occurrences lies
/// in there, whenever it began.
pub(crate) fn calendar_query(first: Date, last: Date) -> String {
    let start = first.strftime("%Y%m%dT000000Z");
    let end = last.tomorrow().unwrap_or(last).strftime("%Y%m%dT000000Z");
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop><d:getetag/><c:calendar-data/></d:prop>
  <c:filter>
    <c:comp-filter name="VCALENDAR">
      <c:comp-filter name="VEVENT">
        <c:time-range start="{start}" end="{end}"/>
      </c:comp-filter>
    </c:comp-filter>
  </c:filter>
</c:calendar-query>"#
    )
}

/// Asks a starting point where the user's calendars are, and whether it is
/// one itself.
pub(crate) const PROPFIND_START: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:a="http://apple.com/ns/ical/">
  <d:prop><d:current-user-principal/><c:calendar-home-set/><d:resourcetype/><d:displayname/><a:calendar-color/><c:supported-calendar-component-set/></d:prop>
</d:propfind>"#;

/// Asks the collections in a calendar home what they are.
pub(crate) const PROPFIND_HOME: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:a="http://apple.com/ns/ical/">
  <d:prop><d:resourcetype/><d:displayname/><a:calendar-color/><c:supported-calendar-component-set/></d:prop>
</d:propfind>"#;

/// One `response` of a multistatus, the little discovery needs of it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Found {
    pub href: String,
    pub name: Option<String>,
    pub color: Option<String>,
    /// The resourcetype says `calendar`.
    pub calendar: bool,
    /// The components it takes, like `VEVENT`; empty when not said.
    pub components: Vec<String>,
    pub principal: Option<String>,
    pub home: Option<String>,
    /// The ETag of a resource.
    pub etag: Option<String>,
    /// The iCalendar text of a resource.
    pub data: Option<String>,
    /// What `current-user-privilege-set` grants, by local name; none when
    /// the answer did not say.
    pub privileges: Option<Vec<String>>,
}

impl Found {
    /// Whether samu may write there (ADR 0068 §1): `write`,
    /// `write-content` or `all`; none when the server did not say.
    pub(crate) fn writable(&self) -> Option<bool> {
        self.privileges.as_ref().map(|p| {
            p.iter()
                .any(|p| matches!(p.as_str(), "write" | "write-content" | "all"))
        })
    }

    /// A collection that holds appointments: a calendar that takes VEVENT,
    /// or does not say what it takes.
    pub(crate) fn holds_events(&self) -> bool {
        self.calendar
            && (self.components.is_empty()
                || self
                    .components
                    .iter()
                    .any(|c| c.eq_ignore_ascii_case("VEVENT")))
    }
}

/// The responses of a multistatus. An `href` counts where it stands right
/// under `response`, under `current-user-principal` or under
/// `calendar-home-set`; other text only under the property it belongs to.
pub(crate) fn responses(xml: &str) -> Result<Vec<Found>, String> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    let mut path: Vec<String> = Vec::new();
    let mut current: Option<Found> = None;
    let mut text = String::new();
    let local = |name: &[u8]| String::from_utf8_lossy(name).into_owned();
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("not a CalDAV answer: {e}"))?;
        match event {
            Event::Eof => break,
            Event::Start(e) => {
                let name = local(e.local_name().as_ref());
                if name == "response" {
                    current = Some(Found::default());
                }
                if name == "current-user-privilege-set"
                    && let Some(found) = current.as_mut()
                {
                    found.privileges.get_or_insert_with(Vec::new);
                }
                text.clear();
                path.push(name);
            }
            Event::Empty(e) => {
                let name = local(e.local_name().as_ref());
                if let Some(found) = current.as_mut() {
                    let parent = path.last().map(String::as_str);
                    if parent == Some("resourcetype") && name == "calendar" {
                        found.calendar = true;
                    }
                    if parent == Some("privilege")
                        && path.iter().any(|p| p == "current-user-privilege-set")
                    {
                        found
                            .privileges
                            .get_or_insert_with(Vec::new)
                            .push(name.clone());
                    }
                    if name == "current-user-privilege-set" {
                        found.privileges.get_or_insert_with(Vec::new);
                    }
                    if parent == Some("supported-calendar-component-set") && name == "comp" {
                        for attribute in e.attributes().flatten() {
                            if attribute.key.local_name().as_ref() == b"name" {
                                let value = attribute
                                    .unescape_value()
                                    .map_err(|e| format!("not a CalDAV answer: {e}"))?;
                                found.components.push(value.into_owned());
                            }
                        }
                    }
                }
            }
            Event::Text(t) => {
                let unescaped = t
                    .unescape()
                    .map_err(|e| format!("not a CalDAV answer: {e}"))?;
                text.push_str(&unescaped);
            }
            Event::CData(c) => text.push_str(&String::from_utf8_lossy(&c.into_inner())),
            Event::End(_) => {
                let Some(name) = path.pop() else { continue };
                let parent = path.last().map(String::as_str);
                let value = text.trim().to_string();
                text.clear();
                if name == "response" {
                    if let Some(found) = current.take() {
                        out.push(found);
                    }
                    continue;
                }
                let Some(found) = current.as_mut() else {
                    continue;
                };
                let some = (!value.is_empty()).then_some(value);
                match (name.as_str(), parent) {
                    ("href", Some("response")) => found.href = some.unwrap_or_default(),
                    ("href", Some("current-user-principal")) => found.principal = some,
                    ("href", Some("calendar-home-set")) => {
                        found.home = found.home.take().or(some);
                    }
                    ("displayname", _) => found.name = some,
                    ("getetag", _) => found.etag = some,
                    ("calendar-data", _) => found.data = some,
                    ("calendar-color", _) => found.color = some.and_then(|c| css_color(&c)),
                    ("calendar", Some("resourcetype")) => found.calendar = true,
                    _ => {}
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// `#rrggbb` from what servers send: `#RRGGBB` or `#RRGGBBAA` (Apple);
/// anything else is no colour.
fn css_color(raw: &str) -> Option<String> {
    let hex = raw.strip_prefix('#')?;
    if !(hex.len() == 6 || hex.len() == 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("#{}", hex[..6].to_ascii_lowercase()))
}

/// The text of every element with this local name, unescaped, in order.
pub(crate) fn texts_of(xml: &str, local: &str) -> Result<Vec<String>, String> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    let mut inside: Option<String> = None;
    loop {
        match reader.read_event() {
            Err(e) => return Err(format!("not a CalDAV answer: {e}")),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) if e.local_name().as_ref() == local.as_bytes() => {
                inside = Some(String::new());
            }
            Ok(Event::End(e)) if e.local_name().as_ref() == local.as_bytes() => {
                if let Some(text) = inside.take() {
                    out.push(text);
                }
            }
            Ok(Event::Text(t)) => {
                if let Some(text) = inside.as_mut() {
                    let unescaped = t
                        .unescape()
                        .map_err(|e| format!("not a CalDAV answer: {e}"))?;
                    text.push_str(&unescaped);
                }
            }
            Ok(Event::CData(c)) => {
                if let Some(text) = inside.as_mut() {
                    text.push_str(&String::from_utf8_lossy(&c.into_inner()));
                }
            }
            Ok(_) => {}
        }
    }
    Ok(out)
}

/// What tells this state of the collection from the next: the ctag, else
/// the sync-token. None when the server keeps neither.
pub(crate) fn version_of(xml: &str) -> Option<String> {
    ["getctag", "sync-token"].iter().find_map(|name| {
        texts_of(xml, name)
            .ok()?
            .into_iter()
            .map(|t| t.trim().to_string())
            .find(|t| !t.is_empty())
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;

    #[test]
    fn the_calendar_data_comes_out_unescaped_whatever_the_prefix() {
        let answer = r#"<?xml version="1.0" encoding="UTF-8"?>
<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:response><D:href>/cal/a.ics</D:href><D:propstat><D:prop>
    <D:getetag>"1"</D:getetag>
    <C:calendar-data>BEGIN:VCALENDAR&#13;
SUMMARY:Tom &amp; Jerry&#13;
END:VCALENDAR</C:calendar-data>
  </D:prop></D:propstat></D:response>
  <D:response><D:href>/cal/b.ics</D:href><D:propstat><D:prop>
    <cal:calendar-data xmlns:cal="urn:ietf:params:xml:ns:caldav"><![CDATA[BEGIN:VCALENDAR
END:VCALENDAR]]></cal:calendar-data>
  </D:prop></D:propstat></D:response>
</D:multistatus>"#;
        let texts = texts_of(answer, "calendar-data").unwrap();
        assert_eq!(texts.len(), 2);
        assert!(
            texts[0].contains("SUMMARY:Tom & Jerry\r\n"),
            "{:?}",
            texts[0]
        );
        assert!(texts[1].starts_with("BEGIN:VCALENDAR\nEND:VCALENDAR"));
    }

    #[test]
    fn the_version_is_the_ctag_else_the_sync_token() {
        let ctag = r#"<d:multistatus xmlns:d="DAV:" xmlns:cs="http://calendarserver.org/ns/">
<d:response><d:propstat><d:prop><cs:getctag>"42"</cs:getctag>
<d:sync-token>http://x/sync/9</d:sync-token></d:prop></d:propstat></d:response></d:multistatus>"#;
        assert_eq!(version_of(ctag).as_deref(), Some("\"42\""));
        let token = r#"<d:multistatus xmlns:d="DAV:"><d:response><d:propstat><d:prop>
<d:sync-token>http://x/sync/9</d:sync-token></d:prop></d:propstat></d:response></d:multistatus>"#;
        assert_eq!(version_of(token).as_deref(), Some("http://x/sync/9"));
        assert_eq!(version_of("<d:multistatus xmlns:d=\"DAV:\"/>"), None);
    }

    #[test]
    fn the_collections_of_a_home_are_read_with_name_colour_and_components() {
        let answer = r##"<?xml version="1.0" encoding="UTF-8"?>
<d:multistatus xmlns:d="DAV:" xmlns:cal="urn:ietf:params:xml:ns:caldav" xmlns:x1="http://apple.com/ns/ical/">
  <d:response><d:href>/dav/calendars/alice/</d:href><d:propstat><d:prop>
    <d:resourcetype><d:collection/></d:resourcetype>
  </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
  <d:response><d:href>/dav/calendars/alice/personal/</d:href><d:propstat><d:prop>
    <d:resourcetype><d:collection/><cal:calendar/></d:resourcetype>
    <d:displayname>Pers&#246;nlich &amp; Familie</d:displayname>
    <x1:calendar-color>#3B82F6FF</x1:calendar-color>
    <cal:supported-calendar-component-set><cal:comp name="VEVENT"/><cal:comp name="VTODO"/></cal:supported-calendar-component-set>
  </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
  <d:response><d:href>/dav/calendars/alice/tasks/</d:href><d:propstat><d:prop>
    <d:resourcetype><d:collection/><cal:calendar/></d:resourcetype>
    <cal:supported-calendar-component-set><cal:comp name="VTODO"/></cal:supported-calendar-component-set>
  </d:prop></d:propstat><d:propstat><d:prop><d:displayname/><x1:calendar-color/></d:prop>
  <d:status>HTTP/1.1 404 Not Found</d:status></d:propstat></d:response>
</d:multistatus>"##;
        let found = responses(answer).unwrap();
        assert_eq!(found.len(), 3);
        assert!(!found[0].calendar);
        assert_eq!(found[1].href, "/dav/calendars/alice/personal/");
        assert_eq!(found[1].name.as_deref(), Some("Persönlich & Familie"));
        assert_eq!(found[1].color.as_deref(), Some("#3b82f6"));
        assert!(found[1].holds_events());
        assert!(
            found[2].calendar && !found[2].holds_events(),
            "{:?}",
            found[2]
        );
        assert_eq!(found[2].name, None);
    }

    #[test]
    fn the_principal_and_the_home_are_hrefs_under_their_property() {
        let answer = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
<d:response><d:href>/</d:href><d:propstat><d:prop>
<d:current-user-principal><d:href>/principals/alice/</d:href></d:current-user-principal>
<c:calendar-home-set><d:href>https://dav.example.org/cal/alice/</d:href></c:calendar-home-set>
<d:resourcetype><d:collection/></d:resourcetype>
</d:prop></d:propstat></d:response></d:multistatus>"#;
        let found = responses(answer).unwrap();
        assert_eq!(found[0].href, "/");
        assert_eq!(found[0].principal.as_deref(), Some("/principals/alice/"));
        assert_eq!(
            found[0].home.as_deref(),
            Some("https://dav.example.org/cal/alice/")
        );
        assert!(!found[0].calendar);
    }

    #[test]
    fn only_six_or_eight_hex_digits_make_a_colour() {
        assert_eq!(css_color("#AABBCC").as_deref(), Some("#aabbcc"));
        assert_eq!(css_color("#aabbccdd").as_deref(), Some("#aabbcc"));
        assert_eq!(css_color("red"), None);
        assert_eq!(css_color("#abc"), None);
        assert_eq!(css_color("#gggggg"), None);
    }

    #[test]
    fn the_query_spans_the_window_with_the_last_day_included() {
        let q = calendar_query(
            jiff::civil::date(2026, 10, 10),
            jiff::civil::date(2027, 12, 14),
        );
        assert!(q.contains(r#"start="20261010T000000Z""#));
        assert!(q.contains(r#"end="20271215T000000Z""#));
    }
}
