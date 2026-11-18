// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a list row shows besides its title (ADR 0130): every field has a
//! place, line 1, line 2 or off, in one fixed order, and the short form
//! is a switch. One layout for every list on this phone, kept in its own
//! cache, never on the server (§7). The web keeps its own in the browser,
//! with the same keys (`web/lib/rowfields.js`).

use serde_json::{Value, json};

use crate::{MobileError, Samu};

/// The fields in order, which is also the order they are cut in (§2).
pub(crate) const ROW_FIELDS: [&str; 12] = [
    "due", "state", "tags", "place", "reason", "defer", "planned", "estimate", "repeat", "note",
    "created", "modified",
];

/// Where the phone starts (§8): the due date beside the title, state and
/// tags below it, short.
fn default_place(key: &str) -> u8 {
    match key {
        "due" => 1,
        "state" | "tags" => 2,
        _ => 0,
    }
}

/// One field and its place: 1, 2, or 0 for off.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RowPlace {
    pub key: String,
    pub place: u8,
}

/// The layout: every field of `ROW_FIELDS` once, in order, and the short
/// form.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RowFields {
    pub places: Vec<RowPlace>,
    pub short: bool,
}

/// What the meta of this phone keeps the layout under.
const META: &str = "row_fields";
/// The switch of ADR 0053, `1` when it was on.
const LEGACY: &str = "details";

/// The layout as the meta keeps it, `{"places": {"due": 1, …}, "short":
/// true}` like the web's `samu.rowfields`.
fn to_text(fields: &RowFields) -> String {
    let places: serde_json::Map<String, Value> = fields
        .places
        .iter()
        .map(|p| (p.key.clone(), json!(p.place)))
        .collect();
    json!({ "places": places, "short": fields.short }).to_string()
}

/// What `to_text` wrote; none when it is not that.
fn from_text(text: &str) -> Option<RowFields> {
    let value: Value = serde_json::from_str(text).ok()?;
    let places = value
        .get("places")?
        .as_object()?
        .iter()
        .filter_map(|(key, place)| {
            Some(RowPlace {
                key: key.clone(),
                place: u8::try_from(place.as_u64()?).ok()?,
            })
        })
        .collect();
    Some(RowFields {
        places,
        short: value.get("short").and_then(Value::as_bool).unwrap_or(true),
    })
}

/// The layout as it counts: every known field once, in order; what the
/// kept one does not say gets its default, an unknown key or a place
/// other than 0, 1, 2 goes. Without a kept layout the old switch
/// „Details“ on adds Start, Geplant and the estimate to line 2.
pub(crate) fn settled(kept: Option<RowFields>, legacy: bool) -> RowFields {
    let place_of = |key: &str| -> u8 {
        if let Some(kept) = &kept {
            if let Some(p) = kept.places.iter().find(|p| p.key == key && p.place <= 2) {
                return p.place;
            }
        } else if legacy && matches!(key, "defer" | "planned" | "estimate") {
            return 2;
        }
        default_place(key)
    };
    RowFields {
        places: ROW_FIELDS
            .iter()
            .map(|key| RowPlace {
                key: (*key).to_owned(),
                place: place_of(key),
            })
            .collect(),
        short: kept.as_ref().is_none_or(|k| k.short),
    }
}

#[uniffi::export]
impl Samu {
    /// The layout of a row on this phone (ADR 0130 §7).
    pub fn row_fields(&self) -> Result<RowFields, MobileError> {
        let cache = self.cache()?;
        let kept = cache.meta(META)?.and_then(|text| from_text(&text));
        let legacy = cache.meta(LEGACY)?.as_deref() == Some("1");
        Ok(settled(kept, legacy))
    }

    /// Keeps the layout, settled, and gives it back.
    pub fn set_row_fields(&self, fields: RowFields) -> Result<RowFields, MobileError> {
        let fields = settled(Some(fields), false);
        self.cache()?.set_meta(META, Some(&to_text(&fields)))?;
        Ok(fields)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places(fields: &RowFields) -> Vec<(&str, u8)> {
        fields
            .places
            .iter()
            .map(|p| (p.key.as_str(), p.place))
            .collect()
    }

    #[test]
    fn a_new_phone_starts_lean_and_short() {
        let fields = settled(None, false);
        assert!(fields.short);
        assert_eq!(fields.places.len(), ROW_FIELDS.len());
        let on: Vec<_> = places(&fields)
            .into_iter()
            .filter(|(_, p)| *p != 0)
            .collect();
        assert_eq!(on, [("due", 1), ("state", 2), ("tags", 2)]);
    }

    #[test]
    fn details_on_adds_defer_planned_and_estimate_once() {
        let fields = settled(None, true);
        let two: Vec<_> = places(&fields)
            .into_iter()
            .filter(|(_, p)| *p == 2)
            .map(|(k, _)| k)
            .collect();
        assert_eq!(two, ["state", "tags", "defer", "planned", "estimate"]);
        // A kept layout wins over the old switch.
        let kept = settled(Some(settled(None, false)), true);
        assert_eq!(kept, settled(None, false));
    }

    #[test]
    fn a_kept_layout_is_settled_in_order() {
        let kept = RowFields {
            places: vec![
                RowPlace {
                    key: "note".into(),
                    place: 1,
                },
                RowPlace {
                    key: "bogus".into(),
                    place: 2,
                },
                RowPlace {
                    key: "tags".into(),
                    place: 7,
                },
                RowPlace {
                    key: "due".into(),
                    place: 0,
                },
            ],
            short: false,
        };
        let fields = settled(Some(kept), false);
        assert!(!fields.short);
        let keys: Vec<_> = fields.places.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, ROW_FIELDS);
        let p = places(&fields);
        assert!(p.contains(&("note", 1)));
        assert!(p.contains(&("due", 0)));
        // An impossible place falls back to the default.
        assert!(p.contains(&("tags", 2)));
    }

    #[test]
    fn the_text_goes_there_and_back() {
        let mut fields = settled(None, true);
        fields.short = false;
        fields.places[2].place = 1;
        assert_eq!(settled(from_text(&to_text(&fields)), false), fields);
        assert!(to_text(&fields).contains(r#""tags":1"#));
        assert_eq!(from_text("{nope"), None);
        assert_eq!(from_text(r#"{"short": true}"#), None);
    }
}
