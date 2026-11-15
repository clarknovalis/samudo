//! Quick capture over `POST /capture` (ADR 0011, ADR 0027): a line of text
//! in, the task it makes out, with what was recognised where.

use serde::{Deserialize, Serialize};

use crate::{Clock, Id, Reminder, Task};

/// What a client sends to `POST /capture`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureRequest {
    pub text: String,
    /// The project when the text names none (the project of the view).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<Id>,
    /// An open task the new one goes under, as its last child and in its
    /// project (ADR 0062 §3). A project named in the text wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Id>,
    /// The scope chosen on the device: its `capture` fills in what the
    /// text names not (ADR 0043 §6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Id>,
    /// A template chosen by a button (ADR 0073 §5); a `/name` in the text
    /// wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<Id>,
    /// An existing task the line applies to instead of making a new one
    /// (ADR 0078 §3): tags are added, single fields overwritten, `// text`
    /// appended to the note. Refused together with `parent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<Id>,
    /// Compute and write nothing, to highlight while one types.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub preview: bool,
}

/// What `POST /capture` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureResponse {
    /// The task as created or edited, or as it would be in a preview.
    pub task: Task,
    #[serde(default)]
    pub created: bool,
    /// Tags made for this task (or that would be made), by title.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub new_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<CaptureMark>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<CaptureWarning>,
    /// The reminders a template gave the task (ADR 0073 §5).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reminders: Vec<Reminder>,
    /// Clocks booked with `z:` (ADR 0081 §2), on the task.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clocks: Vec<Clock>,
}

/// A recognised stretch of the text, as UTF-8 byte offsets (ADR 0027 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureMark {
    pub start: usize,
    pub end: usize,
    pub meaning: CaptureMeaning,
}

/// What a stretch of the text means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMeaning {
    Project,
    Tag,
    Flag,
    /// `!A`, `!B`, `!C` (ADR 0057).
    Priority,
    Defer,
    Due,
    Planned,
    Estimate,
    Repeat,
    Note,
    /// `/name`, a template (ADR 0073 §3).
    Template,
    /// `z:`, time booked (ADR 0081 §2).
    Clock,
    /// `x:`, completed (ADR 0081 §2).
    Completed,
    /// `habit:`, the task is a habit (ADR 0091 §5).
    Habit,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    #[serde(other)]
    Unknown,
}

/// Something the parser did not take; the text in question stays in the
/// title. A code, worded by each surface (ADR 0027 §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "warning", rename_all = "snake_case")]
pub enum CaptureWarning {
    UnknownDate {
        meaning: CaptureMeaning,
        text: String,
    },
    UnknownEstimate {
        text: String,
    },
    UnknownRepeat {
        text: String,
    },
    Twice {
        meaning: CaptureMeaning,
        text: String,
    },
    UnknownProject {
        name: String,
    },
    UnknownTag {
        name: String,
    },
    /// `/name` that is no template; it stays in the title (ADR 0073 §3).
    UnknownTemplate {
        name: String,
    },
    /// Text that is no mark, in a line that edits a task: the title has
    /// its own field there (ADR 0078 §2). An edit with it is refused.
    FreeText {
        text: String,
    },
    /// A `z:` whose clock would end in the future (ADR 0081 §2); it is not
    /// booked.
    ClockInFuture {
        text: String,
    },
    /// A `z:` or `x:` whose value is no time samu reads.
    UnknownClock {
        text: String,
    },
    /// A `habit:` whose value is none of the short forms (ADR 0091 §5).
    UnknownHabit {
        text: String,
    },
    /// A warning from a newer samu (ADR 0015).
    #[serde(other)]
    Unknown,
}
