//! A note or comment as a document tree (ADR 0058), the form of catastr's
//! ADR 0097 with samu's links.
//!
//! The tree holds no markup: raw HTML is a node with its text, shown and
//! never run. Every node carries where it is in the text, in code points,
//! so a surface can change the text at a place (a checkbox) without
//! parsing again.

use serde::{Deserialize, Serialize};

use crate::{Id, ObjectKind};

/// The two formats a note or comment is read as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentFormat {
    Org,
    Markdown,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    #[serde(other, rename = "unknown")]
    Unknown,
}

/// Where a node is in the text, in code points: `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// A rendered text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub format: DocumentFormat,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Heading {
        at: Span,
        level: u8,
        /// An Org TODO keyword such as `TODO` or `DONE`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        keyword: Option<String>,
        /// Whether the keyword is one of the finished states.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        done: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        priority: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tags: Vec<String>,
        content: Vec<Inline>,
    },
    Paragraph {
        at: Span,
        content: Vec<Inline>,
    },
    List {
        at: Span,
        ordered: bool,
        /// The number the first item has, where it is not 1.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<u64>,
        items: Vec<ListItem>,
    },
    Table {
        at: Span,
        /// One per column where the format says so (Markdown).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        align: Vec<Align>,
        rows: Vec<TableRow>,
    },
    /// A code, example or source block, and Markdown's indented code.
    Code {
        at: Span,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
        text: String,
    },
    Quote {
        at: Span,
        blocks: Vec<Block>,
    },
    Center {
        at: Span,
        blocks: Vec<Block>,
    },
    /// Lines kept as they are, with markup inside (Org's `verse`).
    Verse {
        at: Span,
        content: Vec<Inline>,
    },
    /// `#+title: …` and the like.
    Keyword {
        at: Span,
        key: String,
        value: String,
    },
    /// An Org properties drawer.
    Properties {
        at: Span,
        entries: Vec<Property>,
    },
    /// Any other Org drawer, shown folded.
    Drawer {
        at: Span,
        name: String,
        text: String,
    },
    /// An Org comment line.
    Comment {
        at: Span,
        text: String,
    },
    /// Raw HTML from Markdown: shown as text, never run.
    Html {
        at: Span,
        text: String,
    },
    Rule {
        at: Span,
    },
    Footnote {
        at: Span,
        label: String,
        blocks: Vec<Block>,
    },
    /// A node from a newer samu that this reader does not know (ADR 0015):
    /// shown as nothing.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListItem {
    pub at: Span,
    /// The checkbox, where there is one: `true` checked, `false` open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    /// Org's `[-]`, a checkbox partly done.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub partial: bool,
    /// Where the checkbox is, its three characters `[ ]`, so a surface can
    /// switch it in the text (ADR 0058 §3).
    #[serde(rename = "box", default, skip_serializing_if = "Option::is_none")]
    pub checkbox: Option<Span>,
    /// The term of a description list item (`term :: description`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub term: Option<Vec<Inline>>,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    None,
    Left,
    Center,
    Right,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    #[serde(other, rename = "unknown")]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableRow {
    pub at: Span,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub header: bool,
    pub cells: Vec<Vec<Inline>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Property {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Inline {
    Text {
        at: Span,
        text: String,
    },
    Strong {
        at: Span,
        content: Vec<Inline>,
    },
    Emphasis {
        at: Span,
        content: Vec<Inline>,
    },
    Underline {
        at: Span,
        content: Vec<Inline>,
    },
    Strike {
        at: Span,
        content: Vec<Inline>,
    },
    /// Inline code and Org's `=verbatim=` and `~code~`.
    Code {
        at: Span,
        text: String,
    },
    Link {
        at: Span,
        target: String,
        class: LinkClass,
        /// The task a `task` link leads to, where it was found; only for a
        /// task, so that an older client still jumps (ADR 0077 §2).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task: Option<Id>,
        /// Any object a `task` link leads to: a task, project, folder or
        /// tag (ADR 0077 §2).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        object: Option<LinkedObject>,
        content: Vec<Inline>,
    },
    /// An image is never loaded, only shown as a link to its target.
    Image {
        at: Span,
        target: String,
        class: LinkClass,
        alt: String,
    },
    Timestamp {
        at: Span,
        text: String,
    },
    FootnoteRef {
        at: Span,
        label: String,
    },
    /// Raw inline HTML from Markdown: shown as text, never run.
    Html {
        at: Span,
        text: String,
    },
    Break {
        at: Span,
    },
    /// A node from a newer samu that this reader does not know (ADR 0015):
    /// shown as nothing.
    #[serde(other)]
    Unknown,
}

/// The object a `samu:` or `id:` link was found to be (ADR 0077 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedObject {
    pub id: Id,
    pub kind: ObjectKind,
}

/// What a link may do (ADR 0058 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkClass {
    /// `http`, `https`, `mailto`: opens outside samu.
    Web,
    /// `samu:<uuid>` and `id:<…>`: opens the task in samu where it was
    /// found, else shown as text.
    Task,
    /// `catastr:`: an address another app of the device opens (ADR 0075
    /// §5); handed to the system, not followed inside samu.
    App,
    /// Everything else, `elisp:`, `shell:`, `javascript:` and `file:` among
    /// them: shown, never followed.
    Inert,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    #[serde(other, rename = "unknown")]
    Unknown,
}

/// One text to render, `POST /render` (ADR 0058 §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderText {
    pub text: String,
    /// Read the text in this format instead of the one it looks like.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<DocumentFormat>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderRequest {
    pub texts: Vec<RenderText>,
}

/// One document per text, in the order asked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderResponse {
    pub documents: Vec<Document>,
}
