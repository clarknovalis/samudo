// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later OR MIT OR Apache-2.0

//! Dates on a task (ADR 0007, ADR 0014 §4).

use std::fmt;

use jiff::Zoned;
use jiff::civil::Date;
use serde::de::{self, IgnoredAny, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// When `defer`, `due` or `planned` is: one of two different things, never a
/// string to guess at.
///
/// On the wire `{"date": "2026-11-20"}` or
/// `{"at": "2026-11-20T09:00:00+01:00[Europe/Berlin]"}`. Written by hand
/// rather than derived, because serde's derive cannot fall back to
/// [`When::Unknown`] for a key it does not know when that key carries a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum When {
    /// A local date without a zone: valid on that day, wherever one is.
    Date(Date),
    /// An instant with its zone.
    At(Zoned),
    /// A form from a newer samu that this reader does not know (ADR 0015).
    /// Read, never written: the server refuses it. On the wire
    /// `{"unknown": null}`.
    Unknown,
}

impl Serialize for When {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            Self::Date(date) => map.serialize_entry("date", date)?,
            Self::At(at) => map.serialize_entry("at", at)?,
            Self::Unknown => map.serialize_entry("unknown", &())?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for When {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(WhenVisitor)
    }
}

struct WhenVisitor;

impl<'de> Visitor<'de> for WhenVisitor {
    type Value = When;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(r#"an object with one key, "date" or "at""#)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<When, A::Error> {
        let Some(key) = map.next_key::<String>()? else {
            return Err(de::Error::invalid_length(0, &self));
        };
        let when = match key.as_str() {
            "date" => When::Date(map.next_value()?),
            "at" => When::At(map.next_value()?),
            _ => {
                map.next_value::<IgnoredAny>()?;
                When::Unknown
            }
        };
        if map.next_key::<IgnoredAny>()?.is_some() {
            return Err(de::Error::invalid_length(2, &self));
        }
        Ok(when)
    }
}

/// How a task repeats (ADR 0007 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatMode {
    /// The next date comes from the rule, whenever the task was completed
    /// (Org `+`).
    Fixed,
    /// Due is counted from completion (Org `.+`).
    DueAfterCompletion,
    /// Defer is counted from completion.
    DeferAfterCompletion,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    /// Read, never written: the server refuses it.
    #[serde(other)]
    Unknown,
}

/// A repetition: an RRULE (RFC 5545) and a mode.
///
/// The rule is a string here; samu-core parses and checks it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repeat {
    /// `FREQ=WEEKLY;BYDAY=MO`, without the `RRULE:` prefix.
    pub rule: String,
    pub mode: RepeatMode,
    /// With `fixed`: skip to the next date in the future instead of one
    /// after the other (Org `++`).
    #[serde(default)]
    pub catch_up: bool,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;
    use serde_json::json;

    #[test]
    fn a_date_is_tagged_as_date() {
        let when = When::Date(jiff::civil::date(2026, 11, 20));
        assert_eq!(
            serde_json::to_value(&when).unwrap(),
            json!({"date": "2026-11-20"})
        );
    }

    #[test]
    fn an_instant_keeps_its_zone() {
        let at: Zoned = "2026-11-20T09:00:00+01:00[Europe/Berlin]".parse().unwrap();
        let value = serde_json::to_value(When::At(at)).unwrap();
        assert_eq!(
            value,
            json!({"at": "2026-11-20T09:00:00+01:00[Europe/Berlin]"})
        );
    }

    #[test]
    fn an_unknown_form_reads_as_unknown() {
        let when: When = serde_json::from_value(json!({"week": "2026-W47"})).unwrap();
        assert_eq!(when, When::Unknown);
        assert_eq!(
            serde_json::to_value(&when).unwrap(),
            json!({"unknown": null})
        );
    }

    #[test]
    fn two_keys_are_refused() {
        let both = json!({"date": "2026-11-20", "at": "2026-11-20T09:00:00+01:00[Europe/Berlin]"});
        assert!(serde_json::from_value::<When>(both).is_err());
        assert!(serde_json::from_value::<When>(json!({})).is_err());
    }

    #[test]
    fn a_bare_string_is_refused() {
        assert!(serde_json::from_value::<When>(json!("2026-11-20")).is_err());
    }
}
