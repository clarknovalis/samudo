// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a list row shows besides its title (ADR 0130, 0131): line 1 is a
//! list of fields with the title as a fixed piece in it, line 2 another,
//! and what neither names is off; the short form is a switch. One layout
//! for every list on this phone, kept in its own cache, never on the
//! server. The web keeps its own in the browser, with the same keys and
//! the same JSON (`web/lib/rowfields.js`).

use serde_json::{Value, json};

use crate::{MobileError, Samu};

/// Every field a row knows.
pub(crate) const ROW_FIELDS: [&str; 12] = [
    "due", "state", "tags", "place", "reason", "defer", "planned", "estimate", "repeat", "note",
    "created", "modified",
];

/// The fixed piece of line 1.
pub(crate) const TITLE: &str = "title";

/// The layout: line 1 with `title` in it, line 2, and the short form.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RowFields {
    pub line1: Vec<String>,
    pub line2: Vec<String>,
    pub short: bool,
}

/// What the meta of this phone keeps the layout under.
const META: &str = "row_fields";
/// The switch of ADR 0053, `1` when it was on.
const LEGACY: &str = "details";

fn owned(keys: &[&str]) -> Vec<String> {
    keys.iter().map(|k| (*k).to_owned()).collect()
}

/// Where the phone starts (ADR 0131 §6): the state before the title, the
/// due date beside it, tags below, short. The old switch „Details“ on adds
/// Start, Geplant and the estimate to line 2.
fn start(legacy: bool) -> RowFields {
    let mut line2 = owned(&["tags"]);
    if legacy {
        line2.extend(owned(&["defer", "planned", "estimate"]));
    }
    RowFields {
        line1: owned(&["state", TITLE, "due"]),
        line2,
        short: true,
    }
}

/// The layout as it counts: known keys only, each once, the title once in
/// line 1 (at its start when it was missing).
pub(crate) fn settled(fields: RowFields) -> RowFields {
    let mut seen: Vec<String> = Vec::new();
    let mut keep = |list: Vec<String>, title: bool| -> Vec<String> {
        list.into_iter()
            .filter(|key| {
                let known = ROW_FIELDS.contains(&key.as_str()) || (title && key == TITLE);
                let ok = known && !seen.contains(key);
                if ok {
                    seen.push(key.clone());
                }
                ok
            })
            .collect()
    };
    let mut line1 = keep(fields.line1, true);
    if !line1.iter().any(|k| k == TITLE) {
        line1.insert(0, TITLE.to_owned());
    }
    let line2 = keep(fields.line2, false);
    RowFields {
        line1,
        line2,
        short: fields.short,
    }
}

/// The layout as the meta keeps it, `{"line1": [...], "line2": [...],
/// "short": true}` like the web's `samu.rowfields`.
fn to_text(fields: &RowFields) -> String {
    json!({ "line1": fields.line1, "line2": fields.line2, "short": fields.short }).to_string()
}

/// What `to_text` wrote; none when it is not that.
fn from_text(text: &str) -> Option<RowFields> {
    let value: Value = serde_json::from_str(text).ok()?;
    let list = |name: &str| -> Option<Vec<String>> {
        Some(
            value
                .get(name)?
                .as_array()?
                .iter()
                .filter_map(|k| k.as_str().map(str::to_owned))
                .collect(),
        )
    };
    Some(RowFields {
        line1: list("line1")?,
        line2: list("line2").unwrap_or_default(),
        short: value.get("short").and_then(Value::as_bool).unwrap_or(true),
    })
}

#[uniffi::export]
impl Samu {
    /// The layout of a row on this phone (ADR 0131 §7).
    pub fn row_fields(&self) -> Result<RowFields, MobileError> {
        let cache = self.cache()?;
        Ok(match cache.meta(META)?.and_then(|text| from_text(&text)) {
            Some(kept) => settled(kept),
            None => start(cache.meta(LEGACY)?.as_deref() == Some("1")),
        })
    }

    /// Keeps the layout, settled, and gives it back.
    pub fn set_row_fields(&self, fields: RowFields) -> Result<RowFields, MobileError> {
        let fields = settled(fields);
        self.cache()?.set_meta(META, Some(&to_text(&fields)))?;
        Ok(fields)
    }

    /// Where a new phone starts, for „Vorgabe“ in the sheet.
    pub fn row_fields_default(&self) -> RowFields {
        start(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_phone_starts_with_the_state_before_the_title() {
        let fields = start(false);
        assert!(fields.short);
        assert_eq!(fields.line1, ["state", "title", "due"]);
        assert_eq!(fields.line2, ["tags"]);
        assert_eq!(settled(fields.clone()), fields);
    }

    #[test]
    fn details_on_adds_defer_planned_and_estimate() {
        assert_eq!(start(true).line2, ["tags", "defer", "planned", "estimate"]);
    }

    #[test]
    fn a_kept_layout_is_settled() {
        let kept = RowFields {
            line1: owned(&["due", "bogus", "due"]),
            line2: owned(&["due", "note", "title", "tags"]),
            short: false,
        };
        let fields = settled(kept);
        assert_eq!(fields.line1, ["title", "due"]);
        assert_eq!(fields.line2, ["note", "tags"]);
        assert!(!fields.short);
    }

    #[test]
    fn the_text_goes_there_and_back() {
        let mut fields = start(true);
        fields.short = false;
        fields.line1.push("note".into());
        assert_eq!(from_text(&to_text(&fields)), Some(fields.clone()));
        assert!(to_text(&fields).contains(r#""line1":["state","title","due","note"]"#));
        assert_eq!(from_text("{nope"), None);
        // A layout of ADR 0130 was never on a phone; it reads as none.
        assert_eq!(from_text(r#"{"places": {"due": 1}, "short": true}"#), None);
    }
}
