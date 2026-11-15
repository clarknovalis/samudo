//! Just enough of Org to import the open tasks of an agenda (ADR 0002 §4):
//! headings with keyword, priority and tags, the planning line under a
//! heading, drawers, and the in-file keyword lines. Text in, structure out;
//! no I/O.
//!
//! The rules follow catastr ADR 0103 where it reads the same files.

use jiff::civil::{Date, Time};

/// The keywords of a file: open ones and done ones (catastr ADR 0103 §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Keywords {
    pub(crate) open: Vec<String>,
    pub(crate) done: Vec<String>,
}

impl Keywords {
    /// What the author's files use, and what Emacs is set to, when a file says
    /// nothing itself.
    #[must_use]
    pub(crate) fn fallback() -> Self {
        Self::parse("TODO NEXT BACKLOG REVIEW WAITING WORK STRT WAIT HOLD | DONE CANCELED KILL")
    }

    /// One `#+TODO:` value: words, `|` between open and done; without `|`
    /// the last word is the done one, as in Org. Shortcuts in parentheses
    /// (`DONE(d!)`) are cut off.
    #[must_use]
    pub(crate) fn parse(line: &str) -> Self {
        let words: Vec<String> = line
            .split_whitespace()
            .map(|w| w.split('(').next().unwrap_or(w).to_string())
            .filter(|w| !w.is_empty())
            .collect();
        match words.iter().position(|w| w == "|") {
            Some(bar) => Self {
                open: words[..bar].to_vec(),
                done: words[bar + 1..].to_vec(),
            },
            None => {
                let mut open = words;
                let done = open.pop().into_iter().collect();
                Self { open, done }
            }
        }
    }

    fn extend(&mut self, other: Self) {
        self.open.extend(other.open);
        self.done.extend(other.done);
    }
}

/// Where a heading stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    Open,
    Done,
    /// No keyword: a heading, not a task.
    None,
}

/// How a repeater moves the date (ADR 0007 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepeatKind {
    /// `+1w`
    Plus,
    /// `++1w`
    CatchUp,
    /// `.+1w`
    FromDone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Repeater {
    pub(crate) kind: RepeatKind,
    pub(crate) n: u32,
    /// `h`, `d`, `w`, `m` or `y`.
    pub(crate) unit: char,
}

/// An active timestamp of the planning line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stamp {
    pub(crate) date: Date,
    pub(crate) time: Option<Time>,
    /// A range `10:00-11:00` had an end.
    pub(crate) has_end: bool,
    pub(crate) repeat: Option<Repeater>,
    /// `-3d` (or `--3d`): (n, unit).
    pub(crate) warning: Option<(u32, char)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Heading {
    pub(crate) level: usize,
    /// 1-based, as Emacs shows it.
    pub(crate) line: usize,
    pub(crate) keyword: Option<String>,
    pub(crate) state: State,
    pub(crate) priority: Option<char>,
    pub(crate) title: String,
    pub(crate) tags: Vec<String>,
    pub(crate) scheduled: Option<Stamp>,
    pub(crate) deadline: Option<Stamp>,
    /// The `:PROPERTIES:` drawer right under the heading.
    pub(crate) properties: Vec<(String, String)>,
    /// The text up to the next heading, without planning line and drawers.
    pub(crate) body: Vec<String>,
    /// The heading line and the planning line as written, for the note of
    /// an ancestor when this heading is not imported itself.
    pub(crate) raw: Vec<String>,
    /// What could not be read, for a message.
    pub(crate) problems: Vec<String>,
}

impl Heading {
    #[must_use]
    pub(crate) fn property(&self, name: &str) -> Option<&str> {
        self.properties
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OrgFile {
    pub(crate) keywords: Keywords,
    pub(crate) filetags: Vec<String>,
    pub(crate) headings: Vec<Heading>,
}

fn heading_level(line: &str) -> Option<usize> {
    let stars = line.bytes().take_while(|&b| b == b'*').count();
    (stars > 0 && line[stars..].starts_with(' ')).then_some(stars)
}

/// `:a:b:` at the end of a heading, split off the title.
fn split_tags(text: &str) -> (&str, Vec<String>) {
    let trimmed = text.trim_end();
    if let Some(start) = trimmed.rfind(char::is_whitespace) {
        let last = &trimmed[start + 1..];
        let tag_char = |c: char| c.is_alphanumeric() || "_@#%:".contains(c);
        if last.len() > 2
            && last.starts_with(':')
            && last.ends_with(':')
            && last.chars().all(tag_char)
        {
            let tags = last
                .split(':')
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .collect();
            return (trimmed[..start].trim_end(), tags);
        }
    }
    (trimmed, Vec::new())
}

/// Statistics cookies (`[2/5]`, `[40%]`, `[/]`) are counts Org keeps in
/// the title; samu counts itself.
fn strip_cookies(title: &str) -> String {
    let is_cookie = |w: &str| {
        w.len() >= 3
            && w.starts_with('[')
            && w.ends_with(']')
            && w[1..w.len() - 1]
                .chars()
                .all(|c| c.is_ascii_digit() || c == '/' || c == '%')
            && (w.contains('/') || w.contains('%'))
    };
    title
        .split(' ')
        .filter(|w| !is_cookie(w))
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn parse_heading(text: &str, level: usize, line: usize, keywords: &Keywords) -> Heading {
    let rest = text[level..].trim_start();
    let (rest, tags) = split_tags(rest);
    let mut words = rest.splitn(2, ' ');
    let first = words.next().unwrap_or("");
    let (keyword, state, rest) = if keywords.open.iter().any(|k| k == first) {
        (
            Some(first.to_string()),
            State::Open,
            words.next().unwrap_or(""),
        )
    } else if keywords.done.iter().any(|k| k == first) {
        (
            Some(first.to_string()),
            State::Done,
            words.next().unwrap_or(""),
        )
    } else {
        (None, State::None, rest)
    };
    let rest = rest.trim_start();
    let (priority, rest) = match rest.as_bytes() {
        [b'[', b'#', p, b']', ..] if p.is_ascii_alphanumeric() => {
            (Some(char::from(*p)), rest[4..].trim_start())
        }
        _ => (None, rest),
    };
    Heading {
        level,
        line,
        keyword,
        state,
        priority,
        title: strip_cookies(rest),
        tags,
        scheduled: None,
        deadline: None,
        properties: Vec::new(),
        body: Vec::new(),
        problems: Vec::new(),
        raw: vec![text.to_string()],
    }
}

fn parse_repeater(word: &str) -> Option<Repeater> {
    let (kind, rest) = if let Some(r) = word.strip_prefix(".+") {
        (RepeatKind::FromDone, r)
    } else if let Some(r) = word.strip_prefix("++") {
        (RepeatKind::CatchUp, r)
    } else {
        (RepeatKind::Plus, word.strip_prefix('+')?)
    };
    // A habit's `/3d` after the repeater is not ours.
    let rest = rest.split('/').next().unwrap_or(rest);
    let (n, unit) = number_and_unit(rest)?;
    Some(Repeater { kind, n, unit })
}

fn number_and_unit(text: &str) -> Option<(u32, char)> {
    let unit = text.chars().last()?;
    if !"hdwmy".contains(unit) {
        return None;
    }
    let n: u32 = text[..text.len() - 1].parse().ok()?;
    Some((n, unit))
}

/// `<2026-11-03 Tue 19:30-20:00 .+1w -2d>`
fn parse_stamp(text: &str) -> Result<Stamp, String> {
    let inner = text
        .trim()
        .strip_prefix('<')
        .and_then(|t| t.split('>').next())
        .ok_or_else(|| format!("{text:?} is not an active timestamp"))?;
    let mut words = inner.split_whitespace();
    let date: Date = words
        .next()
        .and_then(|d| d.parse().ok())
        .ok_or_else(|| format!("{text:?} has no date"))?;
    let mut stamp = Stamp {
        date,
        time: None,
        has_end: false,
        repeat: None,
        warning: None,
    };
    for word in words {
        if word.starts_with(|c: char| c.is_ascii_digit()) && word.contains(':') {
            let mut parts = word.splitn(2, '-');
            let start = parts.next().unwrap_or(word);
            stamp.time = Some(
                Time::strptime("%H:%M", start)
                    .map_err(|e| format!("{text:?}: time {start:?}: {e}"))?,
            );
            stamp.has_end = parts.next().is_some();
        } else if word.starts_with(['+', '.']) {
            stamp.repeat =
                Some(parse_repeater(word).ok_or_else(|| format!("{text:?}: repeater {word:?}"))?);
        } else if let Some(w) = word.strip_prefix('-') {
            let w = w.strip_prefix('-').unwrap_or(w);
            stamp.warning =
                Some(number_and_unit(w).ok_or_else(|| format!("{text:?}: warning {word:?}"))?);
        }
        // Anything else is the day's name.
    }
    Ok(stamp)
}

/// `SCHEDULED: <…> DEADLINE: <…> CLOSED: […]`
fn is_planning(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("SCHEDULED:") || t.starts_with("DEADLINE:") || t.starts_with("CLOSED:")
}

fn read_planning(line: &str, heading: &mut Heading) {
    for (word, slot) in [("SCHEDULED:", 0), ("DEADLINE:", 1)] {
        let Some(at) = line.find(word) else { continue };
        let after = line[at + word.len()..].trim_start();
        match parse_stamp(after) {
            Ok(stamp) if slot == 0 => heading.scheduled = Some(stamp),
            Ok(stamp) => heading.deadline = Some(stamp),
            Err(problem) => heading.problems.push(problem),
        }
    }
}

/// `:NAME:` alone on a line, other than `:END:`.
fn drawer_start(line: &str) -> bool {
    let t = line.trim();
    t.len() > 2
        && t.starts_with(':')
        && t.ends_with(':')
        && t != ":END:"
        && t[1..t.len() - 1]
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

fn is_end(line: &str) -> bool {
    line.trim().eq_ignore_ascii_case(":END:")
}

/// The value of `#+NAME:` on a line, if it is one.
fn keyword_line<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let rest = line.trim_start().strip_prefix("#+")?;
    let (key, value) = rest.split_once(':')?;
    key.eq_ignore_ascii_case(name).then_some(value.trim())
}

/// Reads a file. The keywords are the file's own `#+TODO:` lines (also
/// `SEQ_TODO`, `TYP_TODO`, all together as in Org), else `fallback`.
#[must_use]
pub(crate) fn parse(text: &str, fallback: &Keywords) -> OrgFile {
    let lines: Vec<&str> = text.lines().collect();
    let mut own: Option<Keywords> = None;
    let mut filetags = Vec::new();
    for line in &lines {
        if heading_level(line).is_some() {
            break;
        }
        for name in ["TODO", "SEQ_TODO", "TYP_TODO"] {
            if let Some(value) = keyword_line(line, name) {
                let parsed = Keywords::parse(value);
                match own.as_mut() {
                    Some(k) => k.extend(parsed),
                    None => own = Some(parsed),
                }
            }
        }
        if let Some(value) = keyword_line(line, "FILETAGS") {
            filetags.extend(
                value
                    .split([':', ' '])
                    .filter(|t| !t.is_empty())
                    .map(str::to_string),
            );
        }
    }
    let keywords = own.unwrap_or_else(|| fallback.clone());

    let mut headings: Vec<Heading> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some(level) = heading_level(lines[i]) else {
            i += 1;
            continue;
        };
        let mut heading = parse_heading(lines[i], level, i + 1, &keywords);
        i += 1;
        if i < lines.len() && is_planning(lines[i]) {
            read_planning(lines[i], &mut heading);
            heading.raw.push(lines[i].to_string());
            i += 1;
        }
        let mut first_drawer = true;
        while i < lines.len() && heading_level(lines[i]).is_none() {
            let line = lines[i];
            if drawer_start(line) {
                let properties = first_drawer && line.trim().eq_ignore_ascii_case(":PROPERTIES:");
                i += 1;
                while i < lines.len() && !is_end(lines[i]) && heading_level(lines[i]).is_none() {
                    if properties {
                        let t = lines[i].trim();
                        if let Some(rest) = t.strip_prefix(':')
                            && let Some((key, value)) = rest.split_once(':')
                        {
                            heading
                                .properties
                                .push((key.to_string(), value.trim().to_string()));
                        }
                    }
                    i += 1;
                }
                if i < lines.len() && is_end(lines[i]) {
                    i += 1;
                }
                first_drawer = false;
                continue;
            }
            if !line.trim().is_empty() {
                first_drawer = false;
            }
            heading.body.push(line.to_string());
            i += 1;
        }
        headings.push(heading);
    }
    OrgFile {
        keywords,
        filetags,
        headings,
    }
}

/// An inactive timestamp alone, like `[2026-04-21 Mon 10:00]`: when an
/// entry was captured.
#[must_use]
pub(crate) fn inactive_stamp(text: &str) -> Option<(Date, Option<Time>)> {
    let inner = text.trim().strip_prefix('[')?.strip_suffix(']')?;
    let mut words = inner.split_whitespace();
    let date: Date = words.next()?.parse().ok()?;
    let time = words
        .find(|w| w.contains(':'))
        .and_then(|w| Time::strptime("%H:%M", w).ok());
    Some((date, time))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;
    use jiff::civil::{date, time};

    fn one(text: &str) -> Heading {
        let file = parse(text, &Keywords::fallback());
        assert_eq!(file.headings.len(), 1, "{:?}", file.headings);
        file.headings.into_iter().next().unwrap()
    }

    #[test]
    fn keywords_split_at_the_bar_or_before_the_last_word() {
        let k = Keywords::parse("TODO(t) NEXT(n!) | DONE(d!) KILL");
        assert_eq!(k.open, ["TODO", "NEXT"]);
        assert_eq!(k.done, ["DONE", "KILL"]);
        let k = Keywords::parse("OPEN FINISHED");
        assert_eq!(k.open, ["OPEN"]);
        assert_eq!(k.done, ["FINISHED"]);
    }

    #[test]
    fn a_heading_has_keyword_priority_title_and_tags() {
        let h = one("** NEXT [#A] server: pkg update   :it:server:\n");
        assert_eq!(h.level, 2);
        assert_eq!(h.keyword.as_deref(), Some("NEXT"));
        assert_eq!(h.state, State::Open);
        assert_eq!(h.priority, Some('A'));
        assert_eq!(h.title, "server: pkg update");
        assert_eq!(h.tags, ["it", "server"]);
    }

    #[test]
    fn a_word_that_is_not_a_keyword_belongs_to_the_title() {
        let h = one("* NIE ORGZLY EINGABE\n");
        assert_eq!(h.state, State::None);
        assert_eq!(h.title, "NIE ORGZLY EINGABE");
        let h = one("* DONE erledigt\n");
        assert_eq!(h.state, State::Done);
    }

    #[test]
    fn cookies_leave_the_title_and_a_colon_in_text_is_not_a_tag() {
        let h = one("* REVIEW [/] admin-tasks :prj:it:\n");
        assert_eq!(h.title, "admin-tasks");
        assert_eq!(h.tags, ["prj", "it"]);
        let h = one("* TODO NoFreshBootEnvironment: on(instance) → x\n");
        assert_eq!(h.title, "NoFreshBootEnvironment: on(instance) → x");
        assert!(h.tags.is_empty());
    }

    #[test]
    fn the_planning_line_gives_dates_times_repeaters_and_warnings() {
        let h = one("* TODO Wochenreview\n\
             SCHEDULED: <2026-05-25 Mon 19:30 .+1w> DEADLINE: <2026-06-01 Mon -3d>\n");
        let s = h.scheduled.unwrap();
        assert_eq!(s.date, date(2026, 5, 25));
        assert_eq!(s.time, Some(time(19, 30, 0, 0)));
        assert_eq!(
            s.repeat,
            Some(Repeater {
                kind: RepeatKind::FromDone,
                n: 1,
                unit: 'w'
            })
        );
        let d = h.deadline.unwrap();
        assert_eq!(d.time, None);
        assert_eq!(d.warning, Some((3, 'd')));
        assert!(h.body.is_empty());
    }

    #[test]
    fn repeaters_of_every_kind() {
        let s = parse_stamp("<2026-01-01 Thu ++2m>").unwrap();
        assert_eq!(s.repeat.unwrap().kind, RepeatKind::CatchUp);
        let s = parse_stamp("<2026-01-01 Thu +1y>").unwrap();
        assert_eq!(s.repeat.unwrap().kind, RepeatKind::Plus);
        let s = parse_stamp("<2026-01-01 Thu 10:00-11:30 .+1d/3d>").unwrap();
        assert!(s.has_end);
        assert_eq!(s.repeat.unwrap().n, 1);
        assert!(parse_stamp("<2026-01-01 Thu +1x>").is_err());
        assert!(parse_stamp("<kein Datum>").is_err());
    }

    #[test]
    fn a_broken_planning_line_is_a_problem_not_a_failure() {
        let h = one("* TODO x\nSCHEDULED: <2026-13-40>\n");
        assert!(h.scheduled.is_none());
        assert_eq!(h.problems.len(), 1);
    }

    #[test]
    fn drawers_leave_the_body_and_properties_are_read() {
        let h = one("* TODO x\n\
             :PROPERTIES:\n\
             :ID:       3f0c2a1e-9d4b\n\
             :CUSTOM_ID: x\n\
             :END:\n\
             :LOGBOOK:\n\
             - State \"DONE\" from \"TODO\" [2026-01-01 Thu]\n\
             :END:\n\
             [2026-04-21 Tue 10:00]\n\
             Text\n");
        assert_eq!(h.property("id"), Some("3f0c2a1e-9d4b"));
        assert_eq!(h.properties.len(), 2);
        assert_eq!(h.body, ["[2026-04-21 Tue 10:00]", "Text"]);
    }

    #[test]
    fn a_file_names_its_own_keywords_and_tags() {
        let file = parse(
            "#+TODO: ENTWURF | BESCHLOSSEN\n#+FILETAGS: :haus:garten:\n\
             * ENTWURF eins\n* TODO zwei\n",
            &Keywords::fallback(),
        );
        assert_eq!(file.filetags, ["haus", "garten"]);
        assert_eq!(file.headings[0].state, State::Open);
        assert_eq!(
            file.headings[1].state,
            State::None,
            "TODO is not a keyword here"
        );
    }

    #[test]
    fn a_line_with_stars_but_no_space_is_text() {
        let file = parse("* TODO a\n**fett**\n", &Keywords::fallback());
        assert_eq!(file.headings.len(), 1);
        assert_eq!(file.headings[0].body, ["**fett**"]);
    }

    #[test]
    fn an_inactive_stamp_alone_is_a_capture_time() {
        assert_eq!(
            inactive_stamp("[2026-04-21 Tue 10:00]"),
            Some((date(2026, 4, 21), Some(time(10, 0, 0, 0))))
        );
        assert_eq!(
            inactive_stamp(" [2026-04-21 Tue] "),
            Some((date(2026, 4, 21), None))
        );
        assert_eq!(inactive_stamp("[2026-04-21 Tue] und Text"), None);
    }
}
