// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A note or comment as a document tree (ADR 0058).
//!
//! The parsers are a copy of catastr's (catastr ADR 0097): Markdown through
//! `pulldown-cmark`, Org as a line based subset. What comes out is the tree
//! from `samu_api::Document`, never HTML; every node carries where it is in
//! the text, in code points.
//!
//! Notes have no media type, so the format is read off the text here, and
//! only here (§2). A link is sorted here too, so `elisp:` stays inert in
//! every surface.

mod markdown;
mod org;

use std::sync::LazyLock;

use regex::Regex;

use crate::api::{
    Align, Block, Document, DocumentFormat, Id, Inline, LinkClass, LinkedObject, ListItem,
    ObjectKind, Property, Span, TableRow,
};

/// Reads a text as what it looks like, or as `format` where one is given.
pub fn render_text(text: &str, format: Option<DocumentFormat>) -> Document {
    render(text, format.unwrap_or_else(|| detect(text)))
}

/// Renders text in a format. A format this samu does not know is read as
/// Markdown.
pub fn render(text: &str, format: DocumentFormat) -> Document {
    let positions = Positions::new(text);
    let (format, blocks) = match format {
        DocumentFormat::Org => (DocumentFormat::Org, org::render(text, &positions)),
        DocumentFormat::Markdown | DocumentFormat::Unknown => {
            (DocumentFormat::Markdown, markdown::render(text, &positions))
        }
    };
    Document { format, blocks }
}

/// Whether a text is Org or Markdown (ADR 0058 §2): whichever has more of
/// its signs; a tie, and a text with neither, is Markdown.
pub fn detect(text: &str) -> DocumentFormat {
    let mut org = 0usize;
    let mut markdown = 0usize;
    let mut fenced = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            markdown += 1;
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        if trimmed.starts_with("#+") || is_drawer_line(trimmed) {
            org += 1;
        } else if is_markdown_heading(line) {
            markdown += 1;
        } else if line.starts_with("**") && is_org_heading(line) {
            org += 1;
        }
        org += line.matches("[[").count();
        markdown += markdown_links(line);
        markdown += line.matches("**").count() / 2;
    }
    if org > markdown {
        DocumentFormat::Org
    } else {
        DocumentFormat::Markdown
    }
}

/// `:PROPERTIES:`, `:END:`, `:LOGBOOK:`: a word in colons alone on a line.
fn is_drawer_line(line: &str) -> bool {
    let line = line.trim_end();
    line.len() > 2
        && line.starts_with(':')
        && line.ends_with(':')
        && line[1..line.len() - 1]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
}

/// `# Title` up to `###### Title`.
fn is_markdown_heading(line: &str) -> bool {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    (1..=6).contains(&hashes) && line[hashes..].starts_with(' ')
}

/// Stars at the start of a line and a blank after them.
fn is_org_heading(line: &str) -> bool {
    let stars = line.chars().take_while(|&c| c == '*').count();
    stars > 0 && line[stars..].starts_with(' ')
}

/// `[text](target)`, counted by its middle.
fn markdown_links(line: &str) -> usize {
    line.match_indices("](")
        .filter(|(at, _)| line[at + 2..].contains(')'))
        .count()
}

/// What a link may do (ADR 0058 §3). Unknown schemes are inert: what is not
/// known to be harmless is not followed.
pub fn link_class(target: &str) -> LinkClass {
    let target = target.trim();
    match target.split_once(':') {
        Some((scheme, _))
            if !scheme.is_empty()
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) =>
        {
            match scheme.to_ascii_lowercase().as_str() {
                "http" | "https" | "mailto" => LinkClass::Web,
                "samu" | "id" => LinkClass::Task,
                "catastr" => LinkClass::App,
                _ => LinkClass::Inert,
            }
        }
        _ => LinkClass::Inert,
    }
}

/// Where a `task` link points, for the one who looks it up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskRef {
    /// `samu:<uuid>`, and `id:<uuid>` that is not an Org id.
    Id(Id),
    /// `id:<x>`: an Org id, as kept in `org_id` (ADR 0023 §5).
    OrgId(String),
}

impl TaskRef {
    /// What a target asks for: for `id:`, the Org id first, then a UUID.
    pub fn of(target: &str) -> Vec<Self> {
        let target = target.trim();
        let Some((scheme, rest)) = target.split_once(':') else {
            return Vec::new();
        };
        let rest = rest.trim();
        match scheme.to_ascii_lowercase().as_str() {
            "samu" => Id::from_uri(target)
                .or_else(|| rest.parse().ok())
                .map(Self::Id)
                .into_iter()
                .collect(),
            "id" if !rest.is_empty() => {
                let mut refs = vec![Self::OrgId(rest.to_string())];
                if let Ok(id) = rest.parse() {
                    refs.push(Self::Id(id));
                }
                refs
            }
            _ => Vec::new(),
        }
    }
}

/// Fills in the object behind each `task` link, from `find`: the server
/// asks the account's database, the app its cache (ADR 0077). A link not
/// found stays without one, and is shown as text.
pub fn resolve_links(document: &mut Document, find: &impl Fn(&TaskRef) -> Option<LinkedObject>) {
    for block in &mut document.blocks {
        resolve_block(block, find);
    }
}

fn resolve_block(block: &mut Block, find: &impl Fn(&TaskRef) -> Option<LinkedObject>) {
    match block {
        Block::Heading { content, .. }
        | Block::Paragraph { content, .. }
        | Block::Verse { content, .. } => resolve_inlines(content, find),
        Block::List { items, .. } => {
            for item in items {
                if let Some(term) = &mut item.term {
                    resolve_inlines(term, find);
                }
                for block in &mut item.blocks {
                    resolve_block(block, find);
                }
            }
        }
        Block::Table { rows, .. } => {
            for row in rows {
                for cell in &mut row.cells {
                    resolve_inlines(cell, find);
                }
            }
        }
        Block::Quote { blocks, .. }
        | Block::Center { blocks, .. }
        | Block::Footnote { blocks, .. } => {
            for block in blocks {
                resolve_block(block, find);
            }
        }
        _ => {}
    }
}

fn resolve_inlines(inlines: &mut [Inline], find: &impl Fn(&TaskRef) -> Option<LinkedObject>) {
    for inline in inlines {
        match inline {
            Inline::Link {
                target,
                class,
                task,
                object,
                content,
                ..
            } => {
                if *class == LinkClass::Task {
                    *object = TaskRef::of(target).iter().find_map(find);
                    // For an older client, which knows only tasks (ADR 0077 §2).
                    *task = object.filter(|o| o.kind == ObjectKind::Task).map(|o| o.id);
                }
                resolve_inlines(content, find);
            }
            Inline::Strong { content, .. }
            | Inline::Emphasis { content, .. }
            | Inline::Underline { content, .. }
            | Inline::Strike { content, .. } => resolve_inlines(content, find),
            _ => {}
        }
    }
}

/// The text with the checkbox at `checkbox` switched (ADR 0058 §6): open
/// becomes checked, checked and partly done become open. `None` when there
/// is no checkbox there, as when the text changed in between.
pub fn toggle_checkbox(text: &str, checkbox: Span) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    if checkbox.end != checkbox.start + 3 || checkbox.end > chars.len() {
        return None;
    }
    let mark: String = chars[checkbox.start..checkbox.end].iter().collect();
    let new = match mark.as_str() {
        "[ ]" | "[-]" => 'x',
        "[x]" | "[X]" => ' ',
        _ => return None,
    };
    let mut chars = chars;
    chars[checkbox.start + 1] = new;
    Some(chars.into_iter().collect())
}

#[allow(clippy::expect_used, reason = "a fixed pattern, covered by the tests")]
static URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^(?:https?://|mailto:)[^\s<>\[\]"]+"#).expect("a fixed pattern that compiles")
});

/// The bare URL `rest` starts with, written without link markup, in both
/// formats: not after a letter or digit, and without the punctuation that
/// ends the sentence around it.
fn bare_url<'a>(before: &str, rest: &'a str) -> Option<&'a str> {
    if before
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric)
    {
        return None;
    }
    let found = URL.find(rest)?;
    let url = rest[..found.end()].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '\'', '"']);
    if url.ends_with("://") || url.ends_with(':') {
        return None;
    }
    Some(url)
}

/// Where an inline is.
fn inline_at(inline: &Inline) -> Span {
    match inline {
        Inline::Text { at, .. }
        | Inline::Strong { at, .. }
        | Inline::Emphasis { at, .. }
        | Inline::Underline { at, .. }
        | Inline::Strike { at, .. }
        | Inline::Code { at, .. }
        | Inline::Link { at, .. }
        | Inline::Image { at, .. }
        | Inline::Timestamp { at, .. }
        | Inline::FootnoteRef { at, .. }
        | Inline::Html { at, .. }
        | Inline::Break { at } => *at,
        Inline::Unknown => Span { start: 0, end: 0 },
    }
}

/// Byte offsets to code points. Built once per text; the parsers work in
/// bytes, the tree speaks code points.
struct Positions {
    /// For every character that is not ASCII, its byte offset and the extra
    /// bytes of all wide characters up to and including it. For mostly
    /// ASCII text this stays short.
    wide: Vec<(usize, usize)>,
}

impl Positions {
    fn new(text: &str) -> Self {
        let mut extra = 0;
        let wide = text
            .char_indices()
            .filter(|(_, c)| !c.is_ascii())
            .map(|(byte, c)| {
                extra += c.len_utf8() - 1;
                (byte, extra)
            })
            .collect();
        Self { wide }
    }

    /// The code point at a byte offset on a character boundary.
    fn at(&self, byte: usize) -> usize {
        let before = self.wide.partition_point(|&(offset, _)| offset < byte);
        let extra = before.checked_sub(1).map_or(0, |last| self.wide[last].1);
        byte - extra
    }

    fn span(&self, start: usize, end: usize) -> Span {
        Span {
            start: self.at(start),
            end: self.at(end),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TASK: &str = "01a1c1d1-be00-7364-932c-f70b4e8c4b53";

    #[test]
    fn positions_count_code_points() {
        let text = "aä€😀b";
        let positions = Positions::new(text);
        let bytes: Vec<usize> = text.char_indices().map(|(byte, _)| byte).collect();
        for (index, byte) in bytes.iter().enumerate() {
            assert_eq!(positions.at(*byte), index);
        }
        assert_eq!(positions.at(text.len()), 5);
    }

    #[test]
    fn link_classes() {
        assert_eq!(link_class("https://example.org"), LinkClass::Web);
        assert_eq!(link_class("MAILTO:a@b.c"), LinkClass::Web);
        assert_eq!(link_class(&format!("samu:{TASK}")), LinkClass::Task);
        assert_eq!(link_class("id:4711"), LinkClass::Task);
        assert_eq!(link_class("ID:4711"), LinkClass::Task);
        assert_eq!(
            link_class("catastr:01936f5e-8a00-7c3e-9b1a-3f2d4c5b6f00#page=3"),
            LinkClass::App
        );
        assert_eq!(link_class("CATASTR:x"), LinkClass::App);
        assert_eq!(link_class("file:../a.org"), LinkClass::Inert);
        assert_eq!(link_class("notes/a.md"), LinkClass::Inert);
        assert_eq!(link_class("#heading"), LinkClass::Inert);
        assert_eq!(link_class("*A heading"), LinkClass::Inert);
        assert_eq!(link_class("elisp:(delete-file \"x\")"), LinkClass::Inert);
        assert_eq!(link_class("shell:rm -rf ~"), LinkClass::Inert);
        assert_eq!(link_class(" JavaScript:alert(1)"), LinkClass::Inert);
        assert_eq!(link_class("data:text/html,x"), LinkClass::Inert);
        // ADR 0075 §5: handed to the app that takes it, no longer inert.
        assert_eq!(link_class("catastr:0193a1b2"), LinkClass::App);
    }

    #[test]
    fn detects_org_and_markdown() {
        let org = [
            "#+title: Plan\nText",
            ":PROPERTIES:\n:ID: x\n:END:\nText",
            "Siehe [[https://x.org][hier]].",
            "** Unterpunkt\nText",
            "- [ ] a\n- [[id:abc][b]]",
        ];
        for text in org {
            assert_eq!(detect(text), DocumentFormat::Org, "{text}");
        }
        let markdown = [
            "",
            "Nur Text.",
            "# Plan\nText",
            "Siehe [hier](https://x.org).",
            "Das ist **wichtig**.",
            "```\n#+not org\n[[nor this]]\n```",
            "- [ ] a\n- [x] b",
            // A tie is Markdown.
            "[[a]] and [b](c)",
            // One star and a blank is a Markdown bullet as well.
            "* eins\n* zwei",
        ];
        for text in markdown {
            assert_eq!(detect(text), DocumentFormat::Markdown, "{text}");
        }
    }

    #[test]
    fn detection_decides_the_parser() {
        let document = render_text("#+title: x\n* Kopf", None);
        assert_eq!(document.format, DocumentFormat::Org);
        assert!(matches!(
            document.blocks[1],
            Block::Heading { level: 1, .. }
        ));
        let document = render_text("#+title: x\n* Kopf", Some(DocumentFormat::Markdown));
        assert_eq!(document.format, DocumentFormat::Markdown);
    }

    #[test]
    fn task_links_resolve_by_id_and_org_id() {
        let known: Id = TASK.parse().unwrap_or_default();
        let text = format!(
            "[[samu:{TASK}][a]] [[id:abc][b]] [[id:{TASK}][c]] [[id:nope][d]] [[elisp:(x)][e]] [[https://x.org][f]]"
        );
        let mut document = render(&text, DocumentFormat::Org);
        let task = LinkedObject {
            id: known,
            kind: ObjectKind::Task,
        };
        resolve_links(&mut document, &|r| match r {
            TaskRef::Id(id) if *id == known => Some(task),
            TaskRef::OrgId(org) if org == "abc" => Some(task),
            _ => None,
        });
        let Block::Paragraph { content, .. } = &document.blocks[0] else {
            panic!("{document:?}")
        };
        let tasks: Vec<Option<Id>> = content
            .iter()
            .filter_map(|inline| match inline {
                Inline::Link { task, .. } => Some(*task),
                _ => None,
            })
            .collect();
        assert_eq!(
            tasks,
            [Some(known), Some(known), Some(known), None, None, None]
        );
    }

    /// ADR 0077: a project is found as well; `task` stays empty for it, so
    /// an older client shows the link as text, and `object` names it.
    #[test]
    fn a_link_to_a_project_carries_the_object_but_no_task() {
        let known: Id = TASK.parse().unwrap_or_default();
        let mut document = render(&format!("[[samu:{TASK}][p]]"), DocumentFormat::Org);
        resolve_links(&mut document, &|_| {
            Some(LinkedObject {
                id: known,
                kind: ObjectKind::Project,
            })
        });
        let Block::Paragraph { content, .. } = &document.blocks[0] else {
            panic!("{document:?}")
        };
        assert!(matches!(
            &content[0],
            Inline::Link { task: None, object: Some(o), .. }
                if o.id == known && o.kind == ObjectKind::Project
        ));
    }

    #[test]
    fn markdown_task_links_resolve() {
        let known: Id = TASK.parse().unwrap_or_default();
        let mut document = render(&format!("**[a](samu:{TASK})**"), DocumentFormat::Markdown);
        resolve_links(&mut document, &|_| {
            Some(LinkedObject {
                id: known,
                kind: ObjectKind::Task,
            })
        });
        let Block::Paragraph { content, .. } = &document.blocks[0] else {
            panic!("{document:?}")
        };
        let Inline::Strong { content, .. } = &content[0] else {
            panic!("{content:?}")
        };
        assert!(matches!(&content[0], Inline::Link { task: Some(id), .. } if *id == known));
    }

    fn first_item(document: &Document, index: usize) -> &ListItem {
        let Block::List { items, .. } = &document.blocks[0] else {
            panic!("{document:?}")
        };
        &items[index]
    }

    #[test]
    fn checkboxes_carry_their_place_and_switch() {
        for format in [DocumentFormat::Markdown, DocumentFormat::Org] {
            let text = "- [ ] ä erst\n- [x] dann\n- ohne";
            let document = render(text, format);
            let open = first_item(&document, 0);
            assert_eq!(open.checked, Some(false), "{format:?}");
            let at = open.checkbox.unwrap_or(Span { start: 0, end: 0 });
            assert_eq!(at, Span { start: 2, end: 5 }, "{format:?}");
            assert_eq!(
                toggle_checkbox(text, at).as_deref(),
                Some("- [x] ä erst\n- [x] dann\n- ohne")
            );
            let done = first_item(&document, 1);
            let at = done.checkbox.unwrap_or(Span { start: 0, end: 0 });
            assert_eq!(at, Span { start: 15, end: 18 }, "{format:?}");
            assert_eq!(
                toggle_checkbox(text, at).as_deref(),
                Some("- [ ] ä erst\n- [ ] dann\n- ohne")
            );
            assert_eq!(first_item(&document, 2).checkbox, None);
        }
    }

    #[test]
    fn org_partial_checkbox_becomes_checked() {
        let text = "- [-] halb";
        let document = render(text, DocumentFormat::Org);
        let item = first_item(&document, 0);
        assert!(item.partial);
        let at = item.checkbox.unwrap_or(Span { start: 0, end: 0 });
        assert_eq!(toggle_checkbox(text, at).as_deref(), Some("- [x] halb"));
    }

    #[test]
    fn a_checkbox_that_moved_is_not_switched() {
        assert_eq!(toggle_checkbox("- [ ] a", Span { start: 0, end: 3 }), None);
        assert_eq!(toggle_checkbox("- [ ] a", Span { start: 5, end: 8 }), None);
        assert_eq!(toggle_checkbox("- [ ] a", Span { start: 2, end: 4 }), None);
    }

    #[test]
    fn raw_html_stays_text() {
        let document = render(
            "<script>alert(1)</script>\n\na <b>x</b>",
            DocumentFormat::Markdown,
        );
        assert!(
            matches!(&document.blocks[0], Block::Html { text, .. } if text.contains("<script>"))
        );
        let Block::Paragraph { content, .. } = &document.blocks[1] else {
            panic!("{document:?}")
        };
        assert!(content.iter().any(|i| matches!(i, Inline::Html { .. })));
    }
}
