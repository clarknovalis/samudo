// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Org mode, a subset (catastr ADR 0097, samu ADR 0058 §2).
//!
//! Line based, like the extractor. What it does not know stays text: a
//! construct it gets wrong should look raw, not vanish.
//!
//! Every piece of text the parser handles is a slice of the source, also
//! after a list item's indentation is cut off, so where a node is in the
//! text follows from where its slice starts.

use std::cell::Cell;
use std::sync::LazyLock;

use regex::Regex;

use super::{Block, Inline, ListItem, Positions, Property, Span, TableRow, bare_url, link_class};

pub(super) fn render(text: &str, positions: &Positions) -> Vec<Block> {
    let context = Context::new(text, positions);
    let lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    context.blocks(&lines)
}

#[allow(clippy::expect_used, reason = "a fixed pattern, covered by the tests")]
static HEADING_TAGS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[ \t]+:([\w@#%:]+):[ \t]*$").expect("a fixed pattern that compiles")
});
#[allow(clippy::expect_used, reason = "a fixed pattern, covered by the tests")]
static LIST_ITEM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([ \t]*)([-+*]|\d+[.)])(?:[ \t]+|$)").expect("a fixed pattern that compiles")
});
#[allow(clippy::expect_used, reason = "a fixed pattern, covered by the tests")]
static DRAWER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^:([\w-]+):$").expect("a fixed pattern that compiles"));
#[allow(clippy::expect_used, reason = "a fixed pattern, covered by the tests")]
static PROPERTY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^:([^:\s]+):(?:[ \t]+(.*))?$").expect("a fixed pattern that compiles")
});
#[allow(clippy::expect_used, reason = "a fixed pattern, covered by the tests")]
static TIMESTAMP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^[<\[]\d{4}-\d{2}-\d{2}[^\]>\n]{0,64}[>\]](?:--[<\[]\d{4}-\d{2}-\d{2}[^\]>\n]{0,64}[>\]])?",
    )
    .expect("a fixed pattern that compiles")
});

/// How far emphasis and a link may reach, in bytes. Without a bound, text
/// full of markers that never close would be searched to its end from each.
const EMPHASIS_REACH: usize = 2048;
const LINK_REACH: usize = 4096;

/// How deep lists, blocks and emphasis nest before the rest is plain text.
/// Keeps the parser's recursion and the JSON depth bounded — serde_json
/// refuses to read past 128.
const MAX_DEPTH: usize = 16;

const IMAGE_EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "gif", "webp", "svg", "avif"];

struct Context<'a> {
    source: &'a str,
    positions: &'a Positions,
    open: Vec<String>,
    done: Vec<String>,
    depth: Cell<usize>,
}

impl<'a> Context<'a> {
    fn new(source: &'a str, positions: &'a Positions) -> Self {
        let mut open = vec!["TODO".to_string()];
        let mut done = vec!["DONE".to_string()];
        // `#+TODO: A B | C D`: before the bar open, after it finished; with
        // no bar the last word is the finished one, as in Org.
        for line in source.lines() {
            let Some((key, value)) = keyword(line.trim_start()) else {
                continue;
            };
            if !matches!(key.as_str(), "todo" | "seq_todo" | "typ_todo") {
                continue;
            }
            let words = |part: &str| -> Vec<String> {
                part.split_whitespace()
                    .map(|word| word.split('(').next().unwrap_or(word).to_string())
                    .filter(|word| !word.is_empty())
                    .collect()
            };
            match value.split_once('|') {
                Some((before, after)) => {
                    open.extend(words(before));
                    done.extend(words(after));
                }
                None => {
                    let mut all = words(&value);
                    if let Some(last) = all.pop() {
                        done.push(last);
                    }
                    open.extend(all);
                }
            }
        }
        Self {
            source,
            positions,
            open,
            done,
            depth: Cell::new(0),
        }
    }

    /// Where a slice of the source starts, in bytes.
    fn offset(&self, slice: &str) -> usize {
        slice.as_ptr() as usize - self.source.as_ptr() as usize
    }

    fn span(&self, slice: &str) -> Span {
        let start = self.offset(slice);
        self.positions.span(start, start + slice.len())
    }

    /// From the start of one slice to the end of another.
    fn between(&self, first: &str, last: &str) -> Span {
        self.positions
            .span(self.offset(first), self.offset(last) + last.len())
    }

    /// The source from the start of one slice to the end of another, as one
    /// slice — for a paragraph over several lines.
    fn joined(&self, first: &str, last: &str) -> &'a str {
        &self.source[self.offset(first)..self.offset(last) + last.len()]
    }

    fn blocks(&self, lines: &[&str]) -> Vec<Block> {
        let first = lines.iter().find(|line| !line.trim().is_empty());
        let last = lines.iter().rev().find(|line| !line.trim().is_empty());
        let (Some(first), Some(last)) = (first, last) else {
            return Vec::new();
        };
        if self.depth.get() >= MAX_DEPTH {
            let text = self.joined(first, last);
            return vec![Block::Paragraph {
                at: self.span(text),
                content: vec![self.text(text)],
            }];
        }
        self.depth.set(self.depth.get() + 1);
        let blocks = self.blocks_within(lines);
        self.depth.set(self.depth.get() - 1);
        blocks
    }

    fn blocks_within(&self, lines: &[&str]) -> Vec<Block> {
        let mut blocks = Vec::new();
        let mut index = 0;
        while index < lines.len() {
            let line = lines[index];
            let trimmed = line.trim_start();
            if trimmed.is_empty() {
                index += 1;
                continue;
            }
            if let Some(block) = self.heading(line) {
                blocks.push(block);
                index += 1;
                continue;
            }
            if let Some((block, next)) = self.special_block(lines, index) {
                blocks.push(block);
                index = next;
                continue;
            }
            if let Some((key, value)) = keyword(trimmed) {
                blocks.push(Block::Keyword {
                    at: self.span(line),
                    key,
                    value,
                });
                index += 1;
                continue;
            }
            if is_comment(trimmed) {
                let start = index;
                while index < lines.len() && is_comment(lines[index].trim_start()) {
                    index += 1;
                }
                let text = lines[start..index]
                    .iter()
                    .map(|line| {
                        let line = line.trim_start();
                        line.strip_prefix("# ").unwrap_or(&line[1..])
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                blocks.push(Block::Comment {
                    at: self.between(lines[start], lines[index - 1]),
                    text,
                });
                continue;
            }
            if let Some((block, next)) = self.drawer(lines, index) {
                blocks.push(block);
                index = next;
                continue;
            }
            if trimmed.starts_with('|') {
                let start = index;
                while index < lines.len() && lines[index].trim_start().starts_with('|') {
                    index += 1;
                }
                blocks.push(self.table(&lines[start..index]));
                continue;
            }
            if trimmed.len() >= 5 && trimmed.trim_end().bytes().all(|byte| byte == b'-') {
                blocks.push(Block::Rule {
                    at: self.span(line),
                });
                index += 1;
                continue;
            }
            if let Some((block, next)) = self.footnote(lines, index) {
                blocks.push(block);
                index = next;
                continue;
            }
            if list_item(line).is_some() {
                let (block, next) = self.list(lines, index);
                blocks.push(block);
                index = next;
                continue;
            }
            let start = index;
            index += 1;
            while index < lines.len() && !self.starts_something(lines, index) {
                index += 1;
            }
            let text = self.joined(lines[start], lines[index - 1]).trim_end();
            blocks.push(Block::Paragraph {
                at: self.span(text),
                content: self.inlines(text),
            });
        }
        blocks
    }

    /// Whether a line ends a paragraph.
    fn starts_something(&self, lines: &[&str], index: usize) -> bool {
        let line = lines[index];
        let trimmed = line.trim_start();
        trimmed.is_empty()
            || self.heading(line).is_some()
            || keyword(trimmed).is_some()
            || block_start(trimmed).is_some()
            || is_comment(trimmed)
            || trimmed.starts_with('|')
            || list_item(line).is_some()
            || self.drawer(lines, index).is_some()
    }

    fn heading(&self, line: &str) -> Option<Block> {
        let stars = line.bytes().take_while(|&byte| byte == b'*').count();
        if stars == 0 {
            return None;
        }
        let rest = &line[stars..];
        if !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
            return None;
        }
        let mut title = rest.trim();
        let mut keyword = None;
        let mut done = false;
        let first = title.split([' ', '\t']).next().unwrap_or("");
        if !first.is_empty() && (self.open.iter().any(|word| word == first)) {
            keyword = Some(first.to_string());
        } else if !first.is_empty() && self.done.iter().any(|word| word == first) {
            keyword = Some(first.to_string());
            done = true;
        }
        if keyword.is_some() {
            title = title[first.len()..].trim_start();
        }
        let mut priority = None;
        if let Some(after) = title.strip_prefix("[#")
            && let Some((inner, rest)) = after.split_once(']')
            && inner.chars().count() == 1
        {
            priority = Some(inner.to_string());
            title = rest.trim_start();
        }
        let mut tags = Vec::new();
        if let Some(found) = HEADING_TAGS.captures(title)
            && let (Some(whole), Some(inner)) = (found.get(0), found.get(1))
        {
            tags = inner
                .as_str()
                .split(':')
                .filter(|tag| !tag.is_empty())
                .map(str::to_string)
                .collect();
            title = &title[..whole.start()];
        }
        Some(Block::Heading {
            at: self.span(line),
            level: u8::try_from(stars).unwrap_or(u8::MAX),
            keyword,
            done,
            priority,
            tags,
            content: self.inlines(title.trim_end()),
        })
    }

    /// `#+begin_x` to `#+end_x`. Without its end it is not a block.
    fn special_block(&self, lines: &[&str], index: usize) -> Option<(Block, usize)> {
        let (name, arguments) = block_start(lines[index].trim_start())?;
        let end = format!("#+end_{name}");
        let close =
            (index + 1..lines.len()).find(|&at| lines[at].trim().eq_ignore_ascii_case(&end))?;
        let inner = &lines[index + 1..close];
        let at = self.between(lines[index], lines[close]);
        let block = match name.as_str() {
            "quote" => Block::Quote {
                at,
                blocks: self.blocks(inner),
            },
            "center" => Block::Center {
                at,
                blocks: self.blocks(inner),
            },
            "verse" => Block::Verse {
                at,
                content: match (inner.first(), inner.last()) {
                    (Some(first), Some(last)) => self.inlines(self.joined(first, last)),
                    _ => Vec::new(),
                },
            },
            _ => Block::Code {
                at,
                language: (name == "src")
                    .then(|| arguments.split_whitespace().next().map(str::to_string))
                    .flatten(),
                text: code_text(inner),
            },
        };
        Some((block, close + 1))
    }

    /// `:NAME:` to `:END:`, before the next heading.
    fn drawer(&self, lines: &[&str], index: usize) -> Option<(Block, usize)> {
        let name = DRAWER
            .captures(lines[index].trim())?
            .get(1)?
            .as_str()
            .to_string();
        if name.eq_ignore_ascii_case("end") {
            return None;
        }
        let close = (index + 1..lines.len())
            .take_while(|&at| self.heading(lines[at]).is_none())
            .find(|&at| lines[at].trim().eq_ignore_ascii_case(":end:"))?;
        let inner = &lines[index + 1..close];
        let at = self.between(lines[index], lines[close]);
        let block = if name.eq_ignore_ascii_case("properties") {
            Block::Properties {
                at,
                entries: inner
                    .iter()
                    .filter_map(|line| {
                        let found = PROPERTY.captures(line.trim())?;
                        Some(Property {
                            key: found.get(1)?.as_str().to_string(),
                            value: found
                                .get(2)
                                .map_or("", |value| value.as_str())
                                .trim()
                                .to_string(),
                        })
                    })
                    .collect(),
            }
        } else {
            Block::Drawer {
                at,
                name,
                text: inner
                    .iter()
                    .map(|line| line.trim())
                    .collect::<Vec<_>>()
                    .join("\n"),
            }
        };
        Some((block, close + 1))
    }

    fn table(&self, lines: &[&str]) -> Block {
        let mut rows = Vec::new();
        let mut separator_after = None;
        for line in lines {
            let trimmed = line.trim();
            if trimmed.starts_with("|-") {
                if separator_after.is_none() && !rows.is_empty() {
                    separator_after = Some(rows.len());
                }
                continue;
            }
            let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
            let inner = inner.strip_suffix('|').unwrap_or(inner);
            rows.push(TableRow {
                at: self.span(trimmed),
                header: false,
                cells: inner
                    .split('|')
                    .map(|cell| self.inlines(cell.trim()))
                    .collect(),
            });
        }
        // Rows above the first rule are the header, if anything is below it.
        if let Some(count) = separator_after
            && count < rows.len()
        {
            for row in &mut rows[..count] {
                row.header = true;
            }
        }
        let at = match (lines.first(), lines.last()) {
            (Some(first), Some(last)) => self.between(first, last),
            _ => Span { start: 0, end: 0 },
        };
        Block::Table {
            at,
            align: Vec::new(),
            rows,
        }
    }

    /// `[fn:label] text` at the start of a line, up to a blank line.
    fn footnote(&self, lines: &[&str], index: usize) -> Option<(Block, usize)> {
        let line = lines[index];
        let after = line.strip_prefix("[fn:")?;
        let (label, rest) = after.split_once(']')?;
        if label.is_empty() || label.contains(char::is_whitespace) {
            return None;
        }
        let mut next = index + 1;
        while next < lines.len() && !self.starts_something(lines, next) {
            next += 1;
        }
        let first = rest.trim_start();
        let last = lines[next - 1];
        let blocks = if next - 1 == index && first.is_empty() {
            Vec::new()
        } else {
            let text = if next - 1 == index {
                first
            } else {
                self.joined(first, last)
            }
            .trim_end();
            vec![Block::Paragraph {
                at: self.span(text),
                content: self.inlines(text),
            }]
        };
        Some((
            Block::Footnote {
                at: self.between(line, last),
                label: label.to_string(),
                blocks,
            },
            next,
        ))
    }

    fn list(&self, lines: &[&str], index: usize) -> (Block, usize) {
        let Some(first) = list_item(lines[index]) else {
            // Only called on an item; an empty list rather than a panic.
            return (
                Block::List {
                    at: self.span(lines[index]),
                    ordered: false,
                    start: None,
                    items: Vec::new(),
                },
                index + 1,
            );
        };
        let mut items = Vec::new();
        let mut at = index;
        let mut last_line = lines[index];
        let mut start = None;
        while let Some(item) = list_item(lines[at])
            .filter(|item| item.indent == first.indent && item.ordered == first.ordered)
        {
            // The item runs over every line indented deeper than its bullet,
            // and over single blank lines between them.
            let mut end = at + 1;
            let mut blanks = 0;
            while end < lines.len() {
                let line = lines[end];
                if line.trim().is_empty() {
                    blanks += 1;
                    if blanks > 1 {
                        break;
                    }
                } else if indent(line) > item.indent {
                    blanks = 0;
                } else {
                    break;
                }
                end += 1;
            }
            let mut content_end = end;
            while content_end > at + 1 && lines[content_end - 1].trim().is_empty() {
                content_end -= 1;
            }
            if items.is_empty() && item.ordered {
                start = item.number.filter(|&number| number != 1);
            }
            items.push(self.list_item(&item, &lines[at + 1..content_end], lines[at]));
            last_line = lines[content_end - 1];
            at = end;
            if at >= lines.len() || blanks > 1 {
                break;
            }
        }
        (
            Block::List {
                at: self.between(lines[index], last_line),
                ordered: first.ordered,
                start,
                items,
            },
            at.max(index + 1),
        )
    }

    fn list_item(&self, item: &Bullet<'a>, more: &[&'a str], line: &str) -> ListItem {
        let mut rest = item.rest;
        if let Some(after) = rest.strip_prefix("[@")
            && let Some((_, after)) = after.split_once(']')
        {
            rest = after.trim_start();
        }
        let mut checked = None;
        let mut partial = false;
        let mut checkbox = None;
        for (mark, state, part) in [
            ("[ ]", false, false),
            ("[X]", true, false),
            ("[x]", true, false),
            ("[-]", false, true),
        ] {
            if let Some(after) = rest.strip_prefix(mark)
                && (after.is_empty() || after.starts_with([' ', '\t']))
            {
                checked = Some(state);
                partial = part;
                checkbox = Some(self.span(&rest[..mark.len()]));
                rest = after.trim_start();
                break;
            }
        }
        let mut term = None;
        if !item.ordered {
            if let Some(position) = rest.find(" :: ") {
                term = Some(self.inlines(rest[..position].trim_end()));
                rest = &rest[position + 4..];
            } else if let Some(stripped) = rest.strip_suffix(" ::") {
                term = Some(self.inlines(stripped.trim_end()));
                rest = &rest[rest.len()..];
            }
        }
        // The continuation without the indentation they share, so a nested
        // list sees its own bullets at the left.
        let cut = more
            .iter()
            .filter(|line| !line.trim().is_empty())
            .map(|line| indent(line))
            .min()
            .unwrap_or(0);
        let mut inner: Vec<&str> = Vec::with_capacity(more.len() + 1);
        inner.push(rest);
        inner.extend(more.iter().map(|line| {
            if line.trim().is_empty() {
                &line[line.len()..]
            } else {
                &line[cut.min(indent(line))..]
            }
        }));
        let last = more.last().copied().unwrap_or(line);
        ListItem {
            at: self.between(line, last),
            checked,
            partial,
            checkbox,
            term,
            blocks: self.blocks(&inner),
        }
    }

    fn text(&self, text: &str) -> Inline {
        Inline::Text {
            at: self.span(text),
            text: text.to_string(),
        }
    }

    fn inlines(&self, text: &str) -> Vec<Inline> {
        if text.is_empty() {
            return Vec::new();
        }
        if self.depth.get() >= MAX_DEPTH {
            return vec![self.text(text)];
        }
        self.depth.set(self.depth.get() + 1);
        let inlines = self.inlines_within(text);
        self.depth.set(self.depth.get() - 1);
        inlines
    }

    fn inlines_within(&self, text: &str) -> Vec<Inline> {
        let mut out = Vec::new();
        let bytes = text.as_bytes();
        let closers = Closers::of(text);
        let mut plain = 0;
        let mut index = 0;
        while index < bytes.len() {
            // Every inline starts with an ASCII byte, and those are never
            // inside a wider character.
            if !bytes[index].is_ascii() {
                index += 1;
                continue;
            }
            let Some((inline, next)) = self.inline_at(text, &closers, index) else {
                index += 1;
                continue;
            };
            if plain < index {
                self.push_text(&mut out, &text[plain..index]);
            }
            out.push(inline);
            index = next;
            plain = next;
        }
        if plain < bytes.len() {
            self.push_text(&mut out, &text[plain..]);
        }
        out
    }

    fn push_text(&self, out: &mut Vec<Inline>, text: &str) {
        out.push(self.text(text));
    }

    /// The inline starting at a byte, if one does. Only ever looks at ASCII
    /// bytes, so every slice it cuts is on a character boundary.
    fn inline_at(&self, text: &str, closers: &Closers, index: usize) -> Option<(Inline, usize)> {
        let rest = &text[index..];
        let byte = rest.as_bytes()[0];
        match byte {
            b'[' if rest.starts_with("[[") => self.link(text, closers, index),
            b'[' if rest.starts_with("[fn:") => {
                let close = rest.bytes().take(256).position(|byte| byte == b']')?;
                let label = &rest[4..close];
                if label.is_empty() || label.contains('\n') {
                    return None;
                }
                Some((
                    Inline::FootnoteRef {
                        at: self.span(&rest[..=close]),
                        label: label.to_string(),
                    },
                    index + close + 1,
                ))
            }
            b'[' | b'<' => {
                let found = TIMESTAMP.find(rest)?;
                let slice = &rest[..found.end()];
                Some((
                    Inline::Timestamp {
                        at: self.span(slice),
                        text: slice.to_string(),
                    },
                    index + found.end(),
                ))
            }
            b'h' | b'm' => {
                let url = bare_url(&text[..index], rest)?;
                Some((
                    Inline::Link {
                        at: self.span(url),
                        target: url.to_string(),
                        class: link_class(url),
                        task: None,
                        object: None,
                        content: vec![Inline::Text {
                            at: self.span(url),
                            text: url.to_string(),
                        }],
                    },
                    index + url.len(),
                ))
            }
            b'\\'
                if rest.starts_with("\\\\")
                    && rest[2..].trim_start_matches([' ', '\t']).starts_with('\n')
                    || rest == "\\\\" =>
            {
                Some((
                    Inline::Break {
                        at: self.span(&rest[..2]),
                    },
                    index + 2,
                ))
            }
            b'*' | b'/' | b'_' | b'=' | b'~' | b'+' => self.emphasis(text, closers, index, byte),
            _ => None,
        }
    }

    /// `[[target][description]]` or `[[target]]`.
    fn link(&self, text: &str, closers: &Closers, index: usize) -> Option<(Inline, usize)> {
        let rest = &text[index..];
        let ends = &closers.link_ends;
        let close = *ends.get(ends.partition_point(|&at| at < index + 2))? - index;
        if close > LINK_REACH {
            return None;
        }
        let inner = &rest[2..close];
        if inner.is_empty() || inner.contains('\n') && !inner.contains("][") {
            return None;
        }
        let (target, description) = match inner.split_once("][") {
            Some((target, description)) => (target, Some(description)),
            None => (inner, None),
        };
        let at = self.span(&rest[..close + 2]);
        let next = index + close + 2;
        let class = link_class(target);
        let target_text = target.trim().to_string();
        if description.is_none() && is_image(target) {
            return Some((
                Inline::Image {
                    at,
                    target: target_text,
                    class,
                    alt: String::new(),
                },
                next,
            ));
        }
        let content = match description {
            Some(description) => self.inlines(description),
            None => vec![Inline::Text {
                at: self.span(target),
                text: target.to_string(),
            }],
        };
        Some((
            Inline::Link {
                at,
                target: target_text,
                class,
                task: None,
                object: None,
                content,
            },
            next,
        ))
    }

    /// Org's emphasis: the marker after the start, a space or one of
    /// `-({'"`, the content not starting or ending in a space, the closing
    /// marker before the end, a space or punctuation.
    fn emphasis(
        &self,
        text: &str,
        closers: &Closers,
        index: usize,
        marker: u8,
    ) -> Option<(Inline, usize)> {
        let before = text[..index].chars().next_back();
        if before.is_some_and(|c| !(c.is_whitespace() || "-({'\"".contains(c))) {
            return None;
        }
        let bytes = text.as_bytes();
        let first = *bytes.get(index + 1)?;
        if first.is_ascii_whitespace() || first == marker {
            return None;
        }
        let candidates = &closers.markers[marker_index(marker)?];
        let close = *candidates.get(candidates.partition_point(|&at| at < index + 2))?;
        // As in Org, emphasis runs over at most one line break; and not far.
        let newlines = closers.newlines.partition_point(|&at| at < close)
            - closers.newlines.partition_point(|&at| at < index);
        if newlines > 1 || close - index > EMPHASIS_REACH {
            return None;
        }
        let inner = &text[index + 1..close];
        let at = self.span(&text[index..=close]);
        let inline = match marker {
            b'=' | b'~' => Inline::Code {
                at,
                text: inner.to_string(),
            },
            b'*' => Inline::Strong {
                at,
                content: self.inlines(inner),
            },
            b'/' => Inline::Emphasis {
                at,
                content: self.inlines(inner),
            },
            b'_' => Inline::Underline {
                at,
                content: self.inlines(inner),
            },
            _ => Inline::Strike {
                at,
                content: self.inlines(inner),
            },
        };
        Some((inline, close + 1))
    }
}

const MARKERS: [u8; 6] = *b"*/_=~+";

fn marker_index(marker: u8) -> Option<usize> {
    MARKERS.iter().position(|&known| known == marker)
}

/// Where emphasis may close, found once per text: so each opening marker
/// costs a binary search and not a walk, however many never close.
struct Closers {
    /// Per marker, the byte offsets of the marker after a non-space and
    /// before the end, a space or punctuation.
    markers: [Vec<usize>; 6],
    newlines: Vec<usize>,
    /// Where a `]]` starts, the end of a link.
    link_ends: Vec<usize>,
}

impl Closers {
    fn of(text: &str) -> Self {
        let bytes = text.as_bytes();
        let mut closers = Self {
            markers: Default::default(),
            newlines: Vec::new(),
            link_ends: Vec::new(),
        };
        for (at, &byte) in bytes.iter().enumerate() {
            if byte == b'\n' {
                closers.newlines.push(at);
                continue;
            }
            if byte == b']' && bytes.get(at + 1) == Some(&b']') {
                closers.link_ends.push(at);
            }
            let Some(marker) = marker_index(byte) else {
                continue;
            };
            if at == 0 || bytes[at - 1].is_ascii_whitespace() {
                continue;
            }
            let after = text[at + 1..].chars().next();
            if after.is_none_or(|c| c.is_whitespace() || "-.,;:!?')}\"[\\".contains(c)) {
                closers.markers[marker].push(at);
            }
        }
        closers
    }
}

struct Bullet<'a> {
    indent: usize,
    ordered: bool,
    number: Option<u64>,
    /// The line after the bullet.
    rest: &'a str,
}

fn list_item(line: &str) -> Option<Bullet<'_>> {
    let found = LIST_ITEM.captures(line)?;
    let spaces = found.get(1)?.as_str().len();
    let bullet = found.get(2)?.as_str();
    // A star at the left edge is a heading.
    if bullet == "*" && spaces == 0 {
        return None;
    }
    let ordered = bullet.as_bytes()[0].is_ascii_digit();
    Some(Bullet {
        indent: spaces,
        ordered,
        number: ordered
            .then(|| bullet[..bullet.len() - 1].parse().ok())
            .flatten(),
        rest: &line[found.get(0)?.end()..],
    })
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// `#+key: value`, the key in lower case. Not `#+begin_…`/`#+end_…`.
fn keyword(trimmed: &str) -> Option<(String, String)> {
    let after = trimmed.strip_prefix("#+")?;
    let (key, value) = after.split_once(':')?;
    if key.is_empty() || key.contains(char::is_whitespace) {
        return None;
    }
    let key = key.to_ascii_lowercase();
    if key.starts_with("begin_") || key.starts_with("end_") {
        return None;
    }
    Some((key, value.trim().to_string()))
}

/// `#+begin_name arguments`: the name in lower case and the arguments.
fn block_start(trimmed: &str) -> Option<(String, &str)> {
    let prefix = trimmed.get(..8)?;
    if !prefix.eq_ignore_ascii_case("#+begin_") {
        return None;
    }
    let rest = &trimmed[8..];
    let (name, arguments) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    if name.is_empty() {
        return None;
    }
    Some((name.to_ascii_lowercase(), arguments.trim()))
}

fn is_comment(trimmed: &str) -> bool {
    trimmed == "#" || trimmed.starts_with("# ")
}

fn is_image(target: &str) -> bool {
    let path = target.trim();
    let path = path.strip_prefix("file:").unwrap_or(path);
    path.rsplit_once('.').is_some_and(|(_, extension)| {
        IMAGE_EXTENSIONS
            .iter()
            .any(|known| known.eq_ignore_ascii_case(extension))
    })
}

/// The lines of a code block without their shared indentation, and without
/// the comma Org puts before a `*` or `#+` that would otherwise be syntax.
fn code_text(lines: &[&str]) -> String {
    let cut = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| indent(line))
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|line| {
            let line = if line.trim().is_empty() {
                ""
            } else {
                &line[cut.min(indent(line))..]
            };
            match line.strip_prefix(',') {
                Some(after) if after.starts_with('*') || after.starts_with("#+") => after,
                _ => line,
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::super::{DocumentFormat, LinkClass, render as render_text};
    use super::*;

    fn blocks(text: &str) -> Vec<Block> {
        render_text(text, DocumentFormat::Org).blocks
    }

    fn texts(inlines: &[Inline]) -> String {
        super::super::markdown::plain(inlines)
    }

    #[test]
    fn heading_with_keyword_priority_and_tags() {
        let found = blocks(
            "#+TODO: IDEA | DROPPED\n* IDEA [#A] Eine *Idee* :a:b:\n** DONE Fertig\n*nicht* fett",
        );
        let Block::Heading {
            level,
            keyword,
            done,
            priority,
            tags,
            content,
            ..
        } = &found[1]
        else {
            panic!("{found:?}")
        };
        assert_eq!(*level, 1);
        assert_eq!(keyword.as_deref(), Some("IDEA"));
        assert!(!done);
        assert_eq!(priority.as_deref(), Some("A"));
        assert_eq!(tags, &["a", "b"]);
        assert_eq!(texts(content), "Eine Idee");
        assert!(matches!(
            &found[2],
            Block::Heading {
                level: 2,
                done: true,
                ..
            }
        ));
        // A star with no space after it is emphasis, not a heading.
        assert!(matches!(&found[3], Block::Paragraph { .. }));
    }

    #[test]
    fn a_keyword_not_declared_is_part_of_the_title() {
        let found = blocks("* REQUEST etwas");
        let Block::Heading {
            keyword, content, ..
        } = &found[0]
        else {
            panic!()
        };
        assert_eq!(*keyword, None);
        assert_eq!(texts(content), "REQUEST etwas");
    }

    #[test]
    fn lists_with_checkboxes_terms_and_nesting() {
        let found = blocks(
            "- [X] eins\n  weiter\n  1. tief\n  2. tiefer\n- [-] halb\n- Wort :: Bedeutung\n\nDanach",
        );
        let Block::List { ordered, items, .. } = &found[0] else {
            panic!("{found:?}")
        };
        assert!(!ordered);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].checked, Some(true));
        assert!(
            matches!(&items[0].blocks[1], Block::List { ordered: true, items, .. } if items.len() == 2)
        );
        assert!(items[1].partial);
        assert_eq!(texts(items[2].term.as_ref().unwrap()), "Wort");
        assert!(matches!(&found[1], Block::Paragraph { .. }));
    }

    #[test]
    fn two_blank_lines_end_a_list() {
        let found = blocks("- a\n\n\n- b");
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn ordered_list_keeps_its_start() {
        let found = blocks("3. drei\n4. vier");
        assert!(matches!(
            &found[0],
            Block::List {
                ordered: true,
                start: Some(3),
                ..
            }
        ));
    }

    #[test]
    fn table_with_header() {
        let found = blocks("| a | b |\n|---+---|\n| 1 | *2* |\n| 3 | 4 |");
        let Block::Table { rows, .. } = &found[0] else {
            panic!()
        };
        assert_eq!(rows.len(), 3);
        assert!(rows[0].header && !rows[1].header);
        assert!(matches!(rows[1].cells[1][0], Inline::Strong { .. }));
    }

    #[test]
    fn blocks_drawers_and_comments() {
        let found = blocks(
            "#+begin_src rust :results none\n  ,* not a heading\n  x\n#+end_src\n\
             #+BEGIN_QUOTE\nZitat\n#+END_QUOTE\n\
             :PROPERTIES:\n:ID: 42\n:EMPTY:\n:END:\n\
             :LOGBOOK:\nCLOCK: x\n:END:\n\
             # eine Notiz\n#+begin_src\nohne Ende",
        );
        assert!(
            matches!(&found[0], Block::Code { language: Some(l), text, .. } if l == "rust" && text == "* not a heading\nx")
        );
        assert!(matches!(&found[1], Block::Quote { blocks, .. } if blocks.len() == 1));
        let Block::Properties { entries, .. } = &found[2] else {
            panic!("{found:?}")
        };
        assert_eq!(entries[0].key, "ID");
        assert_eq!(entries[0].value, "42");
        assert_eq!(entries[1].value, "");
        assert!(
            matches!(&found[3], Block::Drawer { name, text, .. } if name == "LOGBOOK" && text == "CLOCK: x")
        );
        assert!(matches!(&found[4], Block::Comment { text, .. } if text == "eine Notiz"));
        // A block without its end is not swallowed: it stays text.
        assert!(matches!(&found[5], Block::Paragraph { .. }));
    }

    #[test]
    fn inline_markup() {
        let found = blocks(
            "a /b/ _c_ =d= ~e~ +f+ [[https://x.org][X]] [[id:7]] [[bild.png]] [fn:1] <2026-11-01 So>--<2026-11-02 Mo> https://y.org. 2*3*4 a_b_c x\\\\\ny",
        );
        let Block::Paragraph { content, .. } = &found[0] else {
            panic!()
        };
        let kinds: Vec<&str> = content
            .iter()
            .filter(|inline| !matches!(inline, Inline::Text { .. }))
            .map(|inline| match inline {
                Inline::Emphasis { .. } => "emphasis",
                Inline::Underline { .. } => "underline",
                Inline::Code { .. } => "code",
                Inline::Strike { .. } => "strike",
                Inline::Link {
                    class: LinkClass::Web,
                    ..
                } => "web",
                Inline::Link {
                    class: LinkClass::Task,
                    ..
                } => "task",
                Inline::Image { .. } => "image",
                Inline::FootnoteRef { .. } => "footnote",
                Inline::Timestamp { .. } => "timestamp",
                Inline::Break { .. } => "break",
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "emphasis",
                "underline",
                "code",
                "code",
                "strike",
                "web",
                "task",
                "image",
                "footnote",
                "timestamp",
                "web",
                "break"
            ]
        );
        // The full stop after a bare URL is not part of it.
        assert!(content.iter().any(
            |inline| matches!(inline, Inline::Link { target, .. } if target == "https://y.org")
        ));
    }

    #[test]
    fn elisp_and_shell_links_are_inert() {
        let found = blocks("[[elisp:(delete-file \"x\")][harmlos]] [[shell:rm -rf ~]]");
        let Block::Paragraph { content, .. } = &found[0] else {
            panic!()
        };
        let classes: Vec<LinkClass> = content
            .iter()
            .filter_map(|inline| match inline {
                Inline::Link { class, .. } => Some(*class),
                _ => None,
            })
            .collect();
        assert_eq!(classes, [LinkClass::Inert, LinkClass::Inert]);
    }
}
