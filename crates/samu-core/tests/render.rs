//! The rendered document tree (ADR 0058, after catastr ADR 0097): where its nodes are, how deep it
//! goes, and the JSON the web UI and the app read.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

use samu_core::api::{Block, Document, DocumentFormat, Inline};
use samu_core::render::render;
use serde_json::json;

fn inline_texts<'a>(inlines: &'a [Inline], out: &mut Vec<(usize, usize, &'a str)>) {
    for inline in inlines {
        match inline {
            Inline::Text { at, text } => out.push((at.start, at.end, text)),
            Inline::Strong { content, .. }
            | Inline::Emphasis { content, .. }
            | Inline::Underline { content, .. }
            | Inline::Strike { content, .. }
            | Inline::Link { content, .. } => inline_texts(content, out),
            _ => {}
        }
    }
}

fn block_texts<'a>(blocks: &'a [Block], out: &mut Vec<(usize, usize, &'a str)>) {
    for block in blocks {
        match block {
            Block::Heading { content, .. }
            | Block::Paragraph { content, .. }
            | Block::Verse { content, .. } => inline_texts(content, out),
            Block::List { items, .. } => {
                for item in items {
                    if let Some(term) = &item.term {
                        inline_texts(term, out);
                    }
                    block_texts(&item.blocks, out);
                }
            }
            Block::Table { rows, .. } => {
                for row in rows {
                    for cell in &row.cells {
                        inline_texts(cell, out);
                    }
                }
            }
            Block::Quote { blocks, .. }
            | Block::Center { blocks, .. }
            | Block::Footnote { blocks, .. } => block_texts(blocks, out),
            _ => {}
        }
    }
}

/// Every text node is exactly the source at its span, in code points: that
/// is what lets marks and annotations sit on the tree.
fn assert_spans(source: &str, document: &Document) {
    let characters: Vec<char> = source.chars().collect();
    let mut texts = Vec::new();
    block_texts(&document.blocks, &mut texts);
    assert!(!texts.is_empty(), "a check over nothing checks nothing");
    for (start, end, text) in texts {
        let there: String = characters[start..end].iter().collect();
        assert_eq!(there, text, "at {start}..{end}");
    }
}

#[test]
fn org_text_sits_at_its_span() {
    let source = "#+title: Größe\n* TODO Über *alles* :tag:\nText mit /Ümlaut/ und [[id:x][Zïel]]\n\
                  - [ ] Pünkt :: Erklärung\n  weiter 😀\n| ä | ö |\n#+begin_quote\nZitat ß\n#+end_quote\n";
    assert_spans(source, &render(source, DocumentFormat::Org));
}

#[test]
fn markdown_text_sits_at_its_span() {
    let source = "# Über *alles*\n\nText mit _Ümlaut_ und [Zïel](x.md)\n\n- [ ] Pünkt 😀\n  - tiefer\n\n\
                  > Zitat ß\n\n| ä | ö |\n|---|---|\n| 1 | 2 |\n";
    assert_spans(source, &render(source, DocumentFormat::Markdown));
}

/// The test files render and read back.
#[test]
fn demo_and_test_files_render() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut seen = 0;
    for directory in ["testdata"] {
        let mut stack = vec![root.join(directory)];
        while let Some(path) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&path) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let format = match path.extension().and_then(|e| e.to_str()) {
                    Some("org") => DocumentFormat::Org,
                    Some("md") => DocumentFormat::Markdown,
                    _ => continue,
                };
                let Ok(source) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let document = render(&source, format);
                let json = serde_json::to_string(&document).unwrap();
                let back: Document = serde_json::from_str(&json).unwrap();
                assert_eq!(back, document, "{}", path.display());
                seen += 1;
            }
        }
    }
    assert!(seen > 0, "found only {seen} files");
}

/// Nesting stops at a depth the JSON reader still takes; past it the text
/// stays, as text.
#[test]
fn deep_nesting_reads_back() {
    let org: String = (0..400)
        .map(|depth| format!("{}- x{depth}\n", " ".repeat(depth)))
        .collect();
    let emphasis = format!("{}x{}", "*/".repeat(300), "/*".repeat(300));
    let markdown = format!("{}x\n\n{}", "> ".repeat(400), "- ".repeat(400));
    for (source, format) in [
        (org.as_str(), DocumentFormat::Org),
        (emphasis.as_str(), DocumentFormat::Org),
        (markdown.as_str(), DocumentFormat::Markdown),
    ] {
        let document = render(source, format);
        let json = serde_json::to_string(&document).unwrap();
        let back: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(back, document);
    }
    let json = serde_json::to_string(&render(&org, DocumentFormat::Org)).unwrap();
    assert!(json.contains("x399"), "the deepest item is still there");
}

/// Markers that never close must not make the parser walk the text from
/// each of them. A megabyte of them used to take seconds.
#[test]
fn lone_markers_stay_linear() {
    let source = "x *a [[b <2026-01-01 /c _d =e ~f +g ".repeat(30_000);
    let started = std::time::Instant::now();
    let document = render(&source, DocumentFormat::Org);
    assert!(!document.blocks.is_empty());
    // Generous for a debug build on a slow machine; quadratic is minutes.
    assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
}

/// The JSON the web UI and the app read (ADR 0058 §3).
#[test]
fn wire_shape() {
    let document = render(
        "* TODO Titel :t:\n- [X] [[https://x.org][x]]",
        DocumentFormat::Org,
    );
    assert_eq!(
        serde_json::to_value(&document).unwrap(),
        json!({
            "format": "org",
            "blocks": [
                {
                    "type": "heading",
                    "at": {"start": 0, "end": 16},
                    "level": 1,
                    "keyword": "TODO",
                    "tags": ["t"],
                    "content": [{"type": "text", "at": {"start": 7, "end": 12}, "text": "Titel"}]
                },
                {
                    "type": "list",
                    "at": {"start": 17, "end": 43},
                    "ordered": false,
                    "items": [{
                        "at": {"start": 17, "end": 43},
                        "checked": true,
                        "box": {"start": 19, "end": 22},
                        "blocks": [{
                            "type": "paragraph",
                            "at": {"start": 23, "end": 43},
                            "content": [{
                                "type": "link",
                                "at": {"start": 23, "end": 43},
                                "target": "https://x.org",
                                "class": "web",
                                "content": [{"type": "text", "at": {"start": 40, "end": 41}, "text": "x"}]
                            }]
                        }]
                    }]
                }
            ]
        })
    );
}
