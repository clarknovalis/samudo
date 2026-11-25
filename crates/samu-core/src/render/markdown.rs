// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Markdown through `pulldown-cmark` (catastr ADR 0097, samu ADR 0058 §2).
//!
//! The parser hands out events with their byte range; a stack of open
//! frames turns them into the tree.

use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag};

use super::{
    Align, Block, Inline, ListItem, Positions, Span, TableRow, bare_url, inline_at, link_class,
};

/// Open frames at most, the root among them. Each is about two levels of
/// JSON.
const MAX_FRAMES: usize = 32;

pub(super) fn render(text: &str, positions: &Positions) -> Vec<Block> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES;
    let mut stack = vec![Frame::new(Kind::Root, Span { start: 0, end: 0 })];
    for (event, range) in Parser::new_ext(text, options).into_offset_iter() {
        let at = positions.span(range.start, range.end);
        match event {
            Event::Start(tag) => {
                // Past a depth, tags only pass their content up: it keeps
                // the tree within what serde_json reads back (128 levels).
                let kind = if stack.len() >= MAX_FRAMES {
                    Kind::Through
                } else {
                    Kind::of(tag)
                };
                stack.push(Frame::new(kind, at));
            }
            Event::End(_) => {
                // The parser balances its tags, so the root is never closed
                // by an event; should it be, it stays.
                if stack.len() < 2 {
                    continue;
                }
                if let Some(frame) = stack.pop()
                    && let Some(parent) = stack.last_mut()
                {
                    frame.close(parent);
                }
            }
            Event::Text(content) => top(&mut stack).text(at, &content),
            Event::Code(content) => top(&mut stack).inline(Inline::Code {
                at,
                text: content.to_string(),
            }),
            Event::Html(content) | Event::InlineHtml(content) => {
                let frame = top(&mut stack);
                if matches!(frame.kind, Kind::Html) {
                    frame.source.push_str(&content);
                } else {
                    frame.inline(Inline::Html {
                        at,
                        text: content.to_string(),
                    });
                }
            }
            Event::FootnoteReference(label) => top(&mut stack).inline(Inline::FootnoteRef {
                at,
                label: label.to_string(),
            }),
            Event::SoftBreak => top(&mut stack).text(at, "\n"),
            Event::HardBreak => top(&mut stack).inline(Inline::Break { at }),
            Event::Rule => top(&mut stack).block(Block::Rule { at }),
            Event::TaskListMarker(checked) => {
                if let Some(item) = stack
                    .iter_mut()
                    .rev()
                    .find(|frame| matches!(frame.kind, Kind::Item))
                {
                    item.checked = Some(checked);
                    item.checkbox = Some(at);
                }
            }
            // Not enabled, so not expected; their text is not lost.
            Event::InlineMath(content) | Event::DisplayMath(content) => {
                top(&mut stack).text(at, &content);
            }
        }
    }
    let mut root = stack.swap_remove(0);
    root.flush();
    root.blocks
}

#[allow(clippy::expect_used, reason = "an invariant of the loop")]
fn top(stack: &mut [Frame]) -> &mut Frame {
    stack
        .last_mut()
        .expect("the root frame stays on the stack until the end")
}

enum Kind {
    Root,
    Paragraph,
    Heading(u8),
    Quote,
    Code(Option<String>),
    Html,
    List(Option<u64>),
    Item,
    Footnote(String),
    Table(Vec<Align>),
    Head,
    Row,
    Cell,
    Emphasis,
    Strong,
    Strike,
    Link(String),
    Image(String),
    /// A tag this tree has no node for: its content goes to the parent.
    Through,
}

impl Kind {
    fn of(tag: Tag<'_>) -> Self {
        match tag {
            Tag::Paragraph => Self::Paragraph,
            Tag::Heading { level, .. } => Self::Heading(level as u8),
            Tag::BlockQuote(_) => Self::Quote,
            Tag::CodeBlock(CodeBlockKind::Fenced(info)) => Self::Code(
                info.split_whitespace()
                    .next()
                    .filter(|language| !language.is_empty())
                    .map(str::to_string),
            ),
            Tag::CodeBlock(CodeBlockKind::Indented) => Self::Code(None),
            Tag::HtmlBlock => Self::Html,
            Tag::List(start) => Self::List(start),
            Tag::Item => Self::Item,
            Tag::FootnoteDefinition(label) => Self::Footnote(label.to_string()),
            Tag::Table(alignments) => Self::Table(
                alignments
                    .into_iter()
                    .map(|alignment| match alignment {
                        Alignment::None => Align::None,
                        Alignment::Left => Align::Left,
                        Alignment::Center => Align::Center,
                        Alignment::Right => Align::Right,
                    })
                    .collect(),
            ),
            Tag::TableHead => Self::Head,
            Tag::TableRow => Self::Row,
            Tag::TableCell => Self::Cell,
            Tag::Emphasis => Self::Emphasis,
            Tag::Strong => Self::Strong,
            Tag::Strikethrough => Self::Strike,
            Tag::Link { dest_url, .. } => Self::Link(dest_url.to_string()),
            Tag::Image { dest_url, .. } => Self::Image(dest_url.to_string()),
            _ => Self::Through,
        }
    }
}

struct Frame {
    kind: Kind,
    at: Span,
    blocks: Vec<Block>,
    inlines: Vec<Inline>,
    items: Vec<ListItem>,
    rows: Vec<TableRow>,
    cells: Vec<Vec<Inline>>,
    /// The text of a code or HTML block, which is not inline content.
    source: String,
    checked: Option<bool>,
    checkbox: Option<Span>,
}

impl Frame {
    fn new(kind: Kind, at: Span) -> Self {
        Self {
            kind,
            at,
            blocks: Vec::new(),
            inlines: Vec::new(),
            items: Vec::new(),
            rows: Vec::new(),
            cells: Vec::new(),
            source: String::new(),
            checked: None,
            checkbox: None,
        }
    }

    fn text(&mut self, at: Span, text: &str) {
        if matches!(self.kind, Kind::Code(_) | Kind::Html) {
            self.source.push_str(text);
            return;
        }
        // The parser splits text at characters that might have been markup;
        // put the pieces back together.
        if let Some(Inline::Text {
            at: last,
            text: joined,
        }) = self.inlines.last_mut()
            && last.end == at.start
        {
            joined.push_str(text);
            last.end = at.end;
            return;
        }
        self.inlines.push(Inline::Text {
            at,
            text: text.to_string(),
        });
    }

    fn inline(&mut self, inline: Inline) {
        self.inlines.push(inline);
    }

    /// A block after loose text: the text becomes a paragraph first, as in a
    /// tight list item followed by a nested list.
    fn block(&mut self, block: Block) {
        self.flush();
        self.blocks.push(block);
    }

    fn flush(&mut self) {
        if self.inlines.is_empty() {
            return;
        }
        let content = bare_links(std::mem::take(&mut self.inlines));
        let at = Span {
            start: content.first().map_or(0, |inline| inline_at(inline).start),
            end: content.last().map_or(0, |inline| inline_at(inline).end),
        };
        self.blocks.push(Block::Paragraph { at, content });
    }

    /// Hands loose blocks and inlines to the parent as they are.
    fn forward(self, parent: &mut Frame) {
        for block in self.blocks {
            parent.block(block);
        }
        parent.inlines.extend(self.inlines);
    }

    fn close(mut self, parent: &mut Frame) {
        let at = self.at;
        // A link's own words stay as they are; everywhere else a URL in the
        // text becomes a link, as in Org (ADR 0058 §3).
        if !matches!(self.kind, Kind::Link(_) | Kind::Image(_)) {
            self.inlines = bare_links(std::mem::take(&mut self.inlines));
        }
        if matches!(self.kind, Kind::Quote | Kind::Item | Kind::Footnote(_)) {
            self.flush();
        }
        let kind = std::mem::replace(&mut self.kind, Kind::Through);
        match kind {
            // An unknown tag may be inline (superscript) or a block: what it
            // holds goes up as it is.
            Kind::Root | Kind::Through => self.forward(parent),
            Kind::Paragraph => parent.block(Block::Paragraph {
                at,
                content: std::mem::take(&mut self.inlines),
            }),
            Kind::Heading(level) => parent.block(Block::Heading {
                at,
                level,
                keyword: None,
                done: false,
                priority: None,
                tags: Vec::new(),
                content: std::mem::take(&mut self.inlines),
            }),
            Kind::Quote => {
                parent.block(Block::Quote {
                    at,
                    blocks: std::mem::take(&mut self.blocks),
                });
            }
            Kind::Code(language) => {
                if self.source.ends_with('\n') {
                    self.source.pop();
                }
                parent.block(Block::Code {
                    at,
                    language,
                    text: std::mem::take(&mut self.source),
                });
            }
            Kind::Html => parent.block(Block::Html {
                at,
                text: self.source.trim_end_matches('\n').to_string(),
            }),
            Kind::List(start) => {
                parent.block(Block::List {
                    at,
                    ordered: start.is_some(),
                    start: start.filter(|&number| number != 1),
                    items: std::mem::take(&mut self.items),
                });
                // Items past the depth limit arrive as loose content.
                self.forward(parent);
            }
            Kind::Item => {
                parent.items.push(ListItem {
                    at,
                    checked: self.checked,
                    partial: false,
                    checkbox: self.checkbox,
                    term: None,
                    blocks: std::mem::take(&mut self.blocks),
                });
            }
            Kind::Footnote(label) => {
                parent.block(Block::Footnote {
                    at,
                    label,
                    blocks: std::mem::take(&mut self.blocks),
                });
            }
            Kind::Table(align) => {
                parent.block(Block::Table {
                    at,
                    align,
                    rows: std::mem::take(&mut self.rows),
                });
                self.forward(parent);
            }
            Kind::Head | Kind::Row => {
                parent.rows.push(TableRow {
                    at,
                    header: matches!(kind, Kind::Head),
                    cells: std::mem::take(&mut self.cells),
                });
                self.forward(parent);
            }
            Kind::Cell => parent.cells.push(std::mem::take(&mut self.inlines)),
            Kind::Emphasis => parent.inline(Inline::Emphasis {
                at,
                content: std::mem::take(&mut self.inlines),
            }),
            Kind::Strong => parent.inline(Inline::Strong {
                at,
                content: std::mem::take(&mut self.inlines),
            }),
            Kind::Strike => parent.inline(Inline::Strike {
                at,
                content: std::mem::take(&mut self.inlines),
            }),
            Kind::Link(target) => parent.inline(Inline::Link {
                at,
                class: link_class(&target),
                target,
                task: None,
                object: None,
                content: std::mem::take(&mut self.inlines),
            }),
            Kind::Image(target) => parent.inline(Inline::Image {
                at,
                class: link_class(&target),
                target,
                alt: plain(&self.inlines),
            }),
        }
    }
}

/// Text with its bare URLs as links. CommonMark only knows `<https://…>`;
/// a URL pasted as it is was plain text.
///
/// A text's range covers its source, which is longer than the text where an
/// escape or entity was read. Only where both are the same length does each
/// piece get its own range; otherwise every piece keeps the whole one.
fn bare_links(inlines: Vec<Inline>) -> Vec<Inline> {
    let mut out = Vec::with_capacity(inlines.len());
    for inline in inlines {
        let Inline::Text { at, text } = inline else {
            out.push(inline);
            continue;
        };
        let exact = text.chars().count() == at.end.saturating_sub(at.start);
        let span = |start: usize, end: usize| {
            if exact {
                Span {
                    start: at.start + text[..start].chars().count(),
                    end: at.start + text[..end].chars().count(),
                }
            } else {
                at
            }
        };
        let bytes = text.as_bytes();
        let mut plain = 0;
        let mut index = 0;
        while index < bytes.len() {
            // Both schemes start with an ASCII byte, never inside a wider
            // character, so every slice is on a boundary.
            let url = matches!(bytes[index], b'h' | b'm')
                .then(|| bare_url(&text[..index], &text[index..]))
                .flatten();
            let Some(url) = url else {
                index += 1;
                continue;
            };
            if plain < index {
                out.push(Inline::Text {
                    at: span(plain, index),
                    text: text[plain..index].to_string(),
                });
            }
            let end = index + url.len();
            out.push(Inline::Link {
                at: span(index, end),
                class: link_class(url),
                target: url.to_string(),
                task: None,
                object: None,
                content: vec![Inline::Text {
                    at: span(index, end),
                    text: url.to_string(),
                }],
            });
            index = end;
            plain = end;
        }
        if plain == 0 {
            out.push(Inline::Text { at, text });
        } else if plain < bytes.len() {
            out.push(Inline::Text {
                at: span(plain, bytes.len()),
                text: text[plain..].to_string(),
            });
        }
    }
    out
}

/// The text of inlines without their markup, for an image's `alt`.
pub(super) fn plain(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text { text, .. } | Inline::Code { text, .. } => out.push_str(text),
            Inline::Strong { content, .. }
            | Inline::Emphasis { content, .. }
            | Inline::Underline { content, .. }
            | Inline::Strike { content, .. }
            | Inline::Link { content, .. } => out.push_str(&plain(content)),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::super::{DocumentFormat, LinkClass, render as render_text};
    use super::*;

    fn blocks(text: &str) -> Vec<Block> {
        render_text(text, DocumentFormat::Markdown).blocks
    }

    #[test]
    fn tight_list_item_with_a_nested_list() {
        let found = blocks("- [x] eins\n  1. tief\n- zwei");
        let Block::List { items, .. } = &found[0] else {
            panic!("{found:?}")
        };
        assert_eq!(items[0].checked, Some(true));
        assert!(
            matches!(&items[0].blocks[0], Block::Paragraph { content, .. } if plain(content) == "eins")
        );
        assert!(matches!(
            &items[0].blocks[1],
            Block::List { ordered: true, .. }
        ));
        assert_eq!(items[1].checked, None);
    }

    #[test]
    fn raw_html_is_text() {
        let found = blocks("<script>alert(1)</script>\n\na <b>b</b>");
        assert!(
            matches!(&found[0], Block::Html { text, .. } if text == "<script>alert(1)</script>")
        );
        let Block::Paragraph { content, .. } = &found[1] else {
            panic!()
        };
        assert!(
            content
                .iter()
                .any(|inline| matches!(inline, Inline::Html { text, .. } if text == "<b>"))
        );
    }

    #[test]
    fn links_images_and_tables() {
        let found = blocks(
            "[a](javascript:alert(1)) [b](samu:01a1c1d1-be00-7364-932c-f70b4e8c4b53) ![c](https://t.example/p.gif)\n\n| x | y |\n|:-:|--|\n| 1 | 2 |",
        );
        let Block::Paragraph { content, .. } = &found[0] else {
            panic!()
        };
        assert!(matches!(
            &content[0],
            Inline::Link {
                class: LinkClass::Inert,
                ..
            }
        ));
        assert!(matches!(
            &content[2],
            Inline::Link {
                class: LinkClass::Task,
                ..
            }
        ));
        assert!(
            matches!(&content[4], Inline::Image { class: LinkClass::Web, alt, .. } if alt == "c")
        );
        let Block::Table { align, rows, .. } = &found[1] else {
            panic!()
        };
        assert_eq!(align, &[Align::Center, Align::None]);
        assert!(rows[0].header);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn code_blocks_keep_their_language() {
        let found = blocks("```rust ignore\nfn x() {}\n```\n\n    indented");
        assert!(
            matches!(&found[0], Block::Code { language: Some(l), text, .. } if l == "rust" && text == "fn x() {}")
        );
        assert!(
            matches!(&found[1], Block::Code { language: None, text, .. } if text == "indented")
        );
    }

    /// The bare links of inlines, with where they are, one level down too.
    fn links(inlines: &[Inline]) -> Vec<(String, Span)> {
        inlines
            .iter()
            .flat_map(|inline| match inline {
                Inline::Link {
                    target,
                    at,
                    class: LinkClass::Web,
                    ..
                } => vec![(target.clone(), *at)],
                Inline::Strong { content, .. } | Inline::Emphasis { content, .. } => links(content),
                _ => Vec::new(),
            })
            .collect()
    }

    fn paragraph(block: &Block) -> &[Inline] {
        match block {
            Block::Paragraph { content, .. } => content,
            Block::List { items, .. } => paragraph(&items[0].blocks[0]),
            other => panic!("not a paragraph: {other:?}"),
        }
    }

    #[test]
    fn bare_urls_become_links() {
        let text = "Siehe https://x.org/a_b?c=1. Und **mailto:a@b.c** oder (https://y.org).";
        let found = blocks(text);
        let content = paragraph(&found[0]);
        let found = links(content);
        let at = |needle: &str| {
            let start = text[..text.find(needle).unwrap()].chars().count();
            Span {
                start,
                end: start + needle.chars().count(),
            }
        };
        assert_eq!(
            found,
            [
                (
                    "https://x.org/a_b?c=1".to_string(),
                    at("https://x.org/a_b?c=1")
                ),
                ("mailto:a@b.c".to_string(), at("mailto:a@b.c")),
                ("https://y.org".to_string(), at("https://y.org")),
            ]
        );
        assert!(matches!(&content[0], Inline::Text { text, .. } if text == "Siehe "));
        assert!(matches!(&content[2], Inline::Text { text, .. } if text == ". Und "));
    }

    #[test]
    fn bare_urls_after_wide_characters_and_in_lists_keep_their_place() {
        let text = "- [ ] Grüße: https://ä.example/x\n- b";
        let found = blocks(text);
        let start = "- [ ] Grüße: ".chars().count();
        assert_eq!(
            links(paragraph(&found[0])),
            [(
                "https://ä.example/x".to_string(),
                Span {
                    start,
                    end: start + "https://ä.example/x".chars().count()
                }
            )]
        );
    }

    #[test]
    fn only_bare_urls_become_links() {
        // Written as a link, in code, mid-word, or a scheme alone: as before.
        for text in [
            "[https://x.org](https://y.org)",
            "<https://x.org>",
            "`https://x.org`",
            "xhttps://x.org",
            "nur https:// und mailto:",
        ] {
            let found = blocks(text);
            let content = paragraph(&found[0]);
            let bare: Vec<_> = links(content)
                .into_iter()
                .filter(|(target, _)| target == "https://x.org")
                .collect();
            let expected = usize::from(text.starts_with('<'));
            assert_eq!(bare.len(), expected, "{text}: {content:?}");
            assert!(
                !content.iter().any(|inline| matches!(
                    inline,
                    Inline::Link { content, .. } if content.iter().any(|i| matches!(i, Inline::Link { .. }))
                )),
                "{text}"
            );
        }
        assert!(matches!(
            paragraph(&blocks("[https://x.org](https://y.org)")[0])[0],
            Inline::Link { ref target, .. } if target == "https://y.org"
        ));
    }

    #[test]
    fn a_bare_url_in_text_read_from_an_escape_spans_the_whole_text() {
        // `\*` is two characters in the text and one in the tree.
        let text = "a\\* https://x.org";
        let found = blocks(text);
        let content = paragraph(&found[0]);
        let whole = Span {
            start: 0,
            end: text.chars().count(),
        };
        assert!(content.iter().any(
            |inline| matches!(inline, Inline::Link { target, .. } if target == "https://x.org")
        ));
        // Pieces of a text whose length changed carry no wrong positions.
        for inline in content {
            let at = inline_at(inline);
            assert!(at.start >= whole.start && at.end <= whole.end, "{inline:?}");
        }
    }
}
