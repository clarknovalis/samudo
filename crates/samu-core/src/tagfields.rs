// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Changing a tag's workflow fields afterwards: whether it is a workflow,
//! the default workflow, its status, and for a state its key and whether
//! it asks for a note. A workflow and its states have the same fields, so
//! [`role`] says which one a tag is. The command line (which depends on
//! samu-api only) and `web/lib/tagfields.js` write the same rules out
//! again; the cases in `testdata/tagfields/cases.json` hold all three to
//! the same answers.

use serde_json::{Value, json};

use crate::api::{Id, Project, Tag, TagStatus};

/// What a tag is in the workflows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// An exclusive group: its children are the states.
    Workflow,
    /// A child of a workflow.
    State,
    /// Neither.
    Tag,
}

/// What `tag` is among `tags`.
#[must_use]
pub fn role(tags: &[Tag], tag: &Tag) -> Role {
    if tag.exclusive {
        return Role::Workflow;
    }
    match tag.parent.and_then(|p| tags.iter().find(|t| t.id == p)) {
        Some(group) if group.exclusive => Role::State,
        _ => Role::Tag,
    }
}

/// The fields asked for; each `None` stays as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fields {
    pub exclusive: Option<bool>,
    pub default_workflow: Option<bool>,
    pub status: Option<TagStatus>,
    /// `Some(None)` takes the key off.
    pub key: Option<Option<String>>,
    pub ask_note: Option<bool>,
}

/// Why nothing is written. [`Refusal::code`] is the word the shared cases
/// and the web use.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("no tag {0}")]
    NoTag(Id),
    #[error("nothing to set")]
    Nothing,
    #[error("only a workflow can be the default workflow")]
    NoWorkflow,
    #[error("a project uses this workflow; give it another one first")]
    InUse,
    #[error("only the states of a workflow have a key and a note")]
    NotAState,
    #[error("a key is one character")]
    KeyLength,
    #[error("another state of this workflow has this key already")]
    KeyTaken,
}

impl Refusal {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoTag(_) => "unknown",
            Self::Nothing => "nothing",
            Self::NoWorkflow => "no_workflow",
            Self::InUse => "in_use",
            Self::NotAState => "not_a_state",
            Self::KeyLength => "key_length",
            Self::KeyTaken => "key_taken",
        }
    }
}

/// The fields to write, as (tag, field, value): a new default takes over
/// from the one before, a workflow taken back stops being the default,
/// and a value the tag has already writes nothing.
///
/// # Errors
///
/// What would leave a workflow half made: see [`Refusal`].
pub fn changes(
    tags: &[Tag],
    projects: &[Project],
    id: Id,
    fields: &Fields,
) -> Result<Vec<(Id, &'static str, Value)>, Refusal> {
    let Some(tag) = tags.iter().find(|t| t.id == id) else {
        return Err(Refusal::NoTag(id));
    };
    if *fields == Fields::default() {
        return Err(Refusal::Nothing);
    }
    let exclusive = fields.exclusive.unwrap_or(tag.exclusive);
    let mut out = Vec::new();
    if let Some(on) = fields.exclusive
        && on != tag.exclusive
    {
        if !on && projects.iter().any(|p| p.workflow == Some(id)) {
            return Err(Refusal::InUse);
        }
        out.push((id, "exclusive", json!(on)));
        if !on && tag.default_workflow && fields.default_workflow != Some(true) {
            out.push((id, "default_workflow", json!(false)));
        }
    }
    if let Some(on) = fields.default_workflow {
        if on && !exclusive {
            return Err(Refusal::NoWorkflow);
        }
        if on != tag.default_workflow {
            out.push((id, "default_workflow", json!(on)));
        }
        if on {
            for other in tags.iter().filter(|t| t.default_workflow && t.id != id) {
                out.push((other.id, "default_workflow", json!(false)));
            }
        }
    }
    if let Some(status) = fields.status {
        out.push((id, "status", json!(status)));
    }
    if (fields.key.is_some() || fields.ask_note.is_some()) && role(tags, tag) != Role::State {
        return Err(Refusal::NotAState);
    }
    if let Some(key) = &fields.key {
        if let Some(k) = key {
            if k.chars().count() != 1 {
                return Err(Refusal::KeyLength);
            }
            if tags
                .iter()
                .any(|t| t.id != id && t.parent == tag.parent && t.key.as_deref() == Some(k))
            {
                return Err(Refusal::KeyTaken);
            }
        }
        out.push((id, "key", json!(key)));
    }
    if let Some(on) = fields.ask_note {
        out.push((id, "ask_note", json!(on)));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use std::collections::HashMap;

    use super::*;

    /// The cases the command line and the web must give as well.
    #[test]
    fn the_shared_cases_hold() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/tagfields/cases.json"
        );
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let names: HashMap<String, Id> = doc["tags"]
            .as_array()
            .unwrap()
            .iter()
            .chain(doc["projects"].as_array().unwrap())
            .map(|t| (t["id"].as_str().unwrap().to_owned(), Id::new()))
            .collect();
        let id = |v: &Value| names[v.as_str().unwrap()];
        let tags: Vec<Tag> = doc["tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                let mut t = t.clone();
                t["id"] = json!(id(&t["id"]));
                if t["parent"].is_string() {
                    t["parent"] = json!(id(&t["parent"]));
                }
                t["created"] = json!("2026-11-19T08:00:00Z");
                t["modified"] = json!("2026-11-19T08:00:00Z");
                t["status"] = json!("active");
                serde_json::from_value(t).unwrap()
            })
            .collect();
        let projects: Vec<Project> = doc["projects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                serde_json::from_value(json!({
                    "id": id(&p["id"]),
                    "created": "2026-11-19T08:00:00Z",
                    "modified": "2026-11-19T08:00:00Z",
                    "title": "p",
                    "kind": "parallel",
                    "status": "active",
                    "workflow": id(&p["workflow"]),
                }))
                .unwrap()
            })
            .collect();
        let cases = doc["cases"].as_array().unwrap();
        assert!(cases.len() >= 10, "the cases are there");
        for case in cases {
            let what = &case["what"];
            let f = &case["fields"];
            let fields = Fields {
                exclusive: f["exclusive"].as_bool(),
                default_workflow: f["default_workflow"].as_bool(),
                status: f
                    .get("status")
                    .map(|s| serde_json::from_value(s.clone()).unwrap()),
                key: f.get("key").map(|k| k.as_str().map(str::to_owned)),
                ask_note: f["ask_note"].as_bool(),
            };
            let got = changes(&tags, &projects, id(&case["tag"]), &fields);
            match case["error"].as_str() {
                Some(code) => assert_eq!(got.map_err(|e| e.code()), Err(code), "{what}"),
                None => {
                    let want: Vec<(Id, &str, Value)> = case["changes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|c| (id(&c[0]), c[1].as_str().unwrap(), c[2].clone()))
                        .collect();
                    assert_eq!(got.unwrap(), want, "{what}");
                }
            }
        }
        let by = |name: &str| tags.iter().find(|t| t.id == names[name]).unwrap();
        assert_eq!(role(&tags, by("status")), Role::Workflow);
        assert_eq!(role(&tags, by("waiting")), Role::State);
        assert_eq!(role(&tags, by("plain")), Role::Tag);
    }
}
