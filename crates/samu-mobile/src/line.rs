//! The edit dialog on the phone (ADR 0078 §1–3, §6; ADR 0102 §7): the
//! title in its own field, what is set as chips with ✕, and a line in the
//! quick-capture syntax that `samu_core::views::capture` applies to the
//! task, offline, as the server would. Title and removals come first, the
//! line after them, in one write to the outbox.

use jiff::Timestamp;
use samu_api::{CaptureRequest, CaptureWarning, Change, Id, Task};
use samu_core::overlay::{self, Objects};
use serde_json::{Value, json};

use crate::edit::{find, move_changes, set};
use crate::{MobileError, Names, Samu, WarningRow, input, parse_id, priority_text, when_text};

/// One thing set on a task, as the edit dialog shows it: `key` names it
/// for ✕ (`project`, `tag:<id>`, `flagged`, `priority`, `defer`, `due`,
/// `planned`, `estimate`, `repeat`, `habit`, the same keys as the web's);
/// `kind` and `value` as in a capture preview's chip.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EditChip {
    pub key: String,
    pub kind: String,
    pub value: String,
}

/// What the line would add or change: the chips that differ from the
/// task's, a new tag as `new_tag`, `note` when it appends to the note,
/// `reminder` and `clock` per one made; and the parser's warnings, among
/// them `free_text`, which saving refuses.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct EditPreview {
    pub added: Vec<EditChip>,
    pub warnings: Vec<WarningRow>,
}

fn chip(key: impl Into<String>, kind: &str, value: impl Into<String>) -> EditChip {
    EditChip {
        key: key.into(),
        kind: kind.into(),
        value: value.into(),
    }
}

/// The chips of `task`, in the web's order; tags `objects` does not know
/// (new in a preview) are left to the caller.
fn chips_of(task: &Task, objects: &Objects) -> Vec<EditChip> {
    let names = Names::new(objects);
    let mut out = Vec::new();
    if let Some(p) = task.project {
        out.push(chip("project", "project", names.project(p)));
    }
    for tagging in &task.tags {
        if objects.tags.iter().any(|t| t.id == tagging.tag) {
            out.push(chip(
                format!("tag:{}", tagging.tag),
                "tag",
                names.tag_path(tagging.tag),
            ));
        }
    }
    if task.flagged {
        out.push(chip("flagged", "flag", ""));
    }
    if let Some(priority) = task.priority.and_then(priority_text) {
        out.push(chip("priority", "priority", priority));
    }
    for (key, when) in [
        ("defer", &task.schedule.defer),
        ("due", &task.schedule.due),
        ("planned", &task.schedule.planned),
    ] {
        if let Some(when) = when {
            out.push(chip(key, key, when_text(when)));
        }
    }
    if let Some(estimate) = task.estimate {
        out.push(chip("estimate", "estimate", estimate.0.to_string()));
    }
    if let Some(repeat) = &task.schedule.repeat {
        out.push(chip("repeat", "repeat", repeat.rule.clone()));
    }
    if task.habit.is_some() {
        out.push(chip("habit", "habit", ""));
    }
    out
}

/// The changes that take the chip `key` off `task`.
fn removal(
    task: &Task,
    key: &str,
    objects: &Objects,
    stamp: &mut dyn FnMut() -> samu_api::Hlc,
) -> Result<Vec<Change>, MobileError> {
    let id = task.id;
    if let Some(tag) = key.strip_prefix("tag:") {
        let tag = parse_id(tag)?;
        return Ok(if task.tags.iter().any(|t| t.tag == tag) {
            vec![Change::Remove {
                object: id,
                field: "tags".into(),
                element: tag,
                note: None,
                hlc: stamp(),
            }]
        } else {
            Vec::new()
        });
    }
    Ok(match key {
        // Out of its project is into the inbox, with its descendants.
        "project" => move_changes(objects, id, None, stamp),
        "flagged" => vec![set(id, "flagged", json!(false), stamp())],
        "priority" | "defer" | "due" | "planned" | "estimate" | "repeat" | "habit" => {
            vec![set(id, key, Value::Null, stamp())]
        }
        other => return Err(input(format!("no chip {other:?}"))),
    })
}

/// A warning of `POST /capture`'s answer as the app words it.
fn warning_row(warning: &CaptureWarning) -> WarningRow {
    let value = serde_json::to_value(warning).unwrap_or(Value::Null);
    let text = |k: &str| value.get(k).and_then(Value::as_str).map(str::to_owned);
    WarningRow {
        code: text("warning").unwrap_or_default(),
        text: text("text").or_else(|| text("name")).unwrap_or_default(),
    }
}

impl Samu {
    fn edit_request(&self, id: Id, line: String, preview: bool) -> CaptureRequest {
        CaptureRequest {
            text: line,
            project: None,
            parent: None,
            scope: None,
            template: None,
            task: Some(id),
            preview,
        }
    }
}

#[uniffi::export]
impl Samu {
    /// What is set on the task, as chips with ✕ (ADR 0078 §1).
    pub fn edit_chips(&self, id: String) -> Result<Vec<EditChip>, MobileError> {
        let id = parse_id(&id)?;
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        Ok(chips_of(find(&objects, id)?, &objects))
    }

    /// What `line` would do to the task; nothing is written.
    pub fn edit_preview(&self, id: String, line: String) -> Result<EditPreview, MobileError> {
        let id = parse_id(&id)?;
        if line.trim().is_empty() {
            return Ok(EditPreview {
                added: Vec::new(),
                warnings: Vec::new(),
            });
        }
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let (settings, _) = Self::settings(&cache)?;
        let before = find(&objects, id)?.clone();
        let placeholder = samu_api::Hlc {
            wall: 0,
            counter: 0,
            device: String::new(),
        };
        let answer = samu_core::views::capture(
            &objects,
            &settings,
            &self.edit_request(id, line, true),
            Timestamp::now(),
            &placeholder,
            &mut Id::new,
        )
        .map_err(|e| input(e.to_string()))?
        .answer;
        let had = chips_of(&before, &objects);
        let mut added: Vec<EditChip> = chips_of(&answer.task, &objects)
            .into_iter()
            .filter(|c| !had.contains(c))
            .collect();
        for name in &answer.new_tags {
            added.push(chip("new_tag", "new_tag", name.clone()));
        }
        if answer.task.note != before.note {
            added.push(chip("note", "note", ""));
        }
        for _ in &answer.reminders {
            added.push(chip("reminder", "reminder", ""));
        }
        for _ in &answer.clocks {
            added.push(chip("clock", "clock", ""));
        }
        Ok(EditPreview {
            added,
            warnings: answer.warnings.iter().map(warning_row).collect(),
        })
    }

    /// Saves the dialog (ADR 0078 §3): the title when it changed, the
    /// chips `removed` names taken off, then the line applied, in one
    /// write to the outbox. A line with free text is refused.
    pub fn edit_save(
        &self,
        id: String,
        title: String,
        removed: Vec<String>,
        line: String,
    ) -> Result<(), MobileError> {
        let id = parse_id(&id)?;
        let title = title.trim().to_owned();
        let settings = {
            let cache = self.cache()?;
            Self::settings(&cache)?.0
        };
        let request = self.edit_request(id, line, false);
        self.queue(|objects, stamp| {
            let task = find(objects, id)?.clone();
            let mut changes = Vec::new();
            if !title.is_empty() && title != task.title {
                changes.push(set(id, "title", json!(title), stamp()));
            }
            for key in &removed {
                changes.extend(removal(&task, key, objects, stamp)?);
            }
            if request.text.trim().is_empty() {
                return Ok(changes);
            }
            // The line applies to the task as the removals leave it.
            let mut after = objects.clone();
            for change in &changes {
                overlay::apply(&mut after, change).map_err(|e| input(e.to_string()))?;
            }
            let placeholder = stamp();
            let capture = samu_core::views::capture(
                &after,
                &settings,
                &request,
                Timestamp::now(),
                &placeholder,
                &mut Id::new,
            )
            .map_err(|e| input(e.to_string()))?;
            changes.extend(
                capture
                    .changes
                    .into_iter()
                    .map(|change| samu_core::clock::restamp(change, stamp())),
            );
            Ok(changes)
        })
    }
}
