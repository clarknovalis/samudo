// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The search language in German and English (ADR 0105 §3). Both are read
//! always; a chip keeps the German word, its wire form, and is shown in the
//! language of the interface. `web/lib/query.js` holds the same pairs.

/// The language a search is shown in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Lang {
    #[default]
    De,
    En,
}

impl Lang {
    /// `de`, `de-AT` … are German, everything else English, as the web's
    /// „System“ reads `navigator.language`.
    #[must_use]
    pub fn from_code(code: &str) -> Self {
        if code.to_lowercase().starts_with("de") {
            Self::De
        } else {
            Self::En
        }
    }
}

/// The words before the colon: German, English.
const KEYS: &[(&str, &str)] = &[
    ("fällig", "due"),
    ("geplant", "planned"),
    ("start", "defer"),
    ("erledigt", "done"),
    ("erstellt", "created"),
    ("geändert", "modified"),
    ("ordner", "folder"),
    ("zustand", "state"),
    ("ist", "is"),
    ("prio", "prio"),
    ("prop", "prop"),
];

/// The words after the colon or `~`: German, English.
const VALUES: &[(&str, &str)] = &[
    ("heute", "today"),
    ("morgen", "tomorrow"),
    ("gestern", "yesterday"),
    ("woche", "week"),
    ("überfällig", "overdue"),
    ("mit", "any"),
    ("ohne", "none"),
    ("markiert", "flagged"),
    ("verfügbar", "available"),
    ("wartet", "waiting"),
    ("offen", "open"),
    ("erledigt", "done"),
    ("verworfen", "dropped"),
    ("gruppe", "group"),
    ("notiz", "note"),
    ("wiederholt", "repeating"),
    ("geclockt", "clocked"),
    ("blocker", "blocker"),
    ("blockiert", "blocked"),
    ("versteckt", "hidden"),
    ("termin", "appointment"),
];

/// A key in either language (lower case) as the German one; `None` when
/// it is none.
#[must_use]
pub(super) fn canonical_key(key: &str) -> Option<&'static str> {
    match key {
        "faellig" => return Some("fällig"),
        "geaendert" => return Some("geändert"),
        _ => {}
    }
    KEYS.iter()
        .find(|(de, en)| *de == key || *en == key)
        .map(|(de, _)| *de)
}

/// A German key in `lang`.
#[must_use]
pub(super) fn key_in(key: &str, lang: Lang) -> &str {
    match lang {
        Lang::De => key,
        Lang::En => KEYS
            .iter()
            .find(|(de, _)| *de == key)
            .map_or(key, |(_, en)| en),
    }
}

/// `7d`, `-30d` → `7t`, `-30t`; `None` when it is no count of days.
fn days(word: &str, from: char, to: char) -> Option<String> {
    let number = word.strip_suffix(from)?;
    let digits = number.strip_prefix('-').unwrap_or(number);
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| format!("{number}{to}"))
}

/// A value in either language (lower case) as the German one; what is
/// neither, a date or a name, as it is.
#[must_use]
pub(super) fn canonical_value(value: &str) -> String {
    if let Some((de, _)) = VALUES.iter().find(|(_, en)| *en == value) {
        return (*de).to_owned();
    }
    days(value, 'd', 't').unwrap_or_else(|| value.to_owned())
}

/// A German value in `lang`.
#[must_use]
pub(super) fn value_in(value: &str, lang: Lang) -> String {
    match lang {
        Lang::De => value.to_owned(),
        Lang::En => VALUES
            .iter()
            .find(|(de, _)| *de == value)
            .map(|(_, en)| (*en).to_owned())
            .or_else(|| days(value, 't', 'd'))
            .unwrap_or_else(|| value.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_languages_read_as_german() {
        assert_eq!(canonical_key("due"), Some("fällig"));
        assert_eq!(canonical_key("fällig"), Some("fällig"));
        assert_eq!(canonical_key("faellig"), Some("fällig"));
        assert_eq!(canonical_key("is"), Some("ist"));
        assert_eq!(canonical_key("folder"), Some("ordner"));
        assert_eq!(canonical_key("nope"), None);
        assert_eq!(canonical_value("today"), "heute");
        assert_eq!(canonical_value("flagged"), "markiert");
        assert_eq!(canonical_value("-30d"), "-30t");
        assert_eq!(canonical_value("7t"), "7t");
        assert_eq!(canonical_value("2026-11-01"), "2026-11-01");
    }

    #[test]
    fn german_is_shown_in_either() {
        assert_eq!(key_in("geändert", Lang::En), "modified");
        assert_eq!(key_in("geändert", Lang::De), "geändert");
        assert_eq!(value_in("überfällig", Lang::En), "overdue");
        assert_eq!(value_in("-7t", Lang::En), "-7d");
        assert_eq!(value_in("<15m", Lang::En), "<15m");
        assert_eq!(value_in("heute", Lang::De), "heute");
        assert_eq!(Lang::from_code("de-AT"), Lang::De);
        assert_eq!(Lang::from_code("en-US"), Lang::En);
        assert_eq!(Lang::from_code("fr"), Lang::En);
    }

    #[test]
    fn every_pair_goes_there_and_back() {
        for (de, _) in VALUES {
            assert_eq!(canonical_value(&value_in(de, Lang::En)), *de);
        }
        for (de, _) in KEYS {
            assert_eq!(canonical_key(key_in(de, Lang::En)), Some(*de));
        }
    }
}
