// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The search field of the phone (ADR 0103 §3): what the list under it
//! offers, the words that become chips in it, and the last searches of this
//! phone. The offers come from samu-core, as in the web; the search itself
//! stays one text in the search language and runs through `Samu::search`.

use samu_api::{ScopeChip, ScopeChipKind};
use samu_core::availability::Snapshot;
use samu_core::query::{self, Lang, OfferAction, OfferOptions};
use serde_json::Value;

use crate::{MobileError, Samu, input};

/// How many searches this phone remembers.
const RECENT: usize = 5;
/// The cache's key for them.
const RECENT_KEY: &str = "recent_searches";
/// The cache's key for the language chosen on this phone (ADR 0105 §2).
const LANG_KEY: &str = "ui_lang";

/// One chip of the search field.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SearchChip {
    /// The kind as on the wire: `project`, `tag`, `due`, `is`, …
    pub kind: String,
    /// An id for projects, tags, folders and states, else the word's value.
    pub value: String,
    pub not: bool,
    /// The word that makes it, `-@Büro` or `#"Mein Projekt"`; the search
    /// is these words and the free text.
    pub word: String,
    /// The word without quotes, to show.
    pub label: String,
}

/// One line of the list under the field: either a chip or a text that
/// replaces the word at the cursor.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SearchOffer {
    pub label: String,
    pub chip: Option<SearchChip>,
    pub insert: Option<String>,
}

/// A typed text taken apart: the chips of its finished words, and the rest
/// with the word still being typed.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SearchSplit {
    pub chips: Vec<SearchChip>,
    pub rest: String,
}

fn kind_name(kind: ScopeChipKind) -> String {
    match serde_json::to_value(kind) {
        Ok(Value::String(name)) => name,
        _ => "unknown".to_owned(),
    }
}

pub(crate) fn row(chip: &ScopeChip, snapshot: &Snapshot<'_>, lang: Lang) -> SearchChip {
    let word = query::words_in(std::slice::from_ref(chip), snapshot, lang);
    SearchChip {
        kind: kind_name(chip.kind),
        value: chip.value.clone(),
        not: chip.not,
        label: word.replace('"', ""),
        word,
    }
}

impl Samu {
    /// The language the app set, German until it does.
    pub(crate) fn ui_lang(&self) -> Lang {
        self.lang.lock().map_or(Lang::De, |lang| *lang)
    }
}

#[uniffi::export]
impl Samu {
    /// The language chosen on this phone: `system`, `de` or `en`
    /// (ADR 0105 §2); `system` when none is.
    pub fn lang_choice(&self) -> Result<String, MobileError> {
        Ok(self
            .cache()?
            .meta(LANG_KEY)?
            .unwrap_or_else(|| "system".to_owned()))
    }

    /// Keeps the language chosen on this phone; another word than
    /// `system`, `de` or `en` is an input error.
    pub fn set_lang_choice(&self, choice: String) -> Result<(), MobileError> {
        let value = match choice.as_str() {
            "system" => None,
            "de" | "en" => Some(choice.as_str()),
            _ => return Err(input(format!("no language {choice}"))),
        };
        self.cache()?.set_meta(LANG_KEY, value)?;
        Ok(())
    }

    /// The language the app shows, resolved from the choice and the
    /// phone's locale: `de…` is German, everything else English. The
    /// words of the search field follow it; the chips stay German.
    pub fn set_lang(&self, code: String) {
        if let Ok(mut lang) = self.lang.lock() {
            *lang = Lang::from_code(&code);
        }
    }

    /// What the list under the search field offers for `word`, the word at
    /// the cursor, from the phone's mirror, offline too (ADR 0103 §3). The
    /// phone does not search appointments, so `ist:termin` is not offered.
    pub fn search_offers(&self, word: String) -> Result<Vec<SearchOffer>, MobileError> {
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let snapshot = objects.snapshot();
        let options = OfferOptions {
            lang: self.ui_lang(),
            ..OfferOptions::default()
        };
        let lang = options.lang;
        Ok(query::offers(&word, &snapshot, options)
            .into_iter()
            .map(|offer| match offer.action {
                OfferAction::Chip(chip) => SearchOffer {
                    label: offer.label,
                    chip: Some(row(&chip, &snapshot, lang)),
                    insert: None,
                },
                OfferAction::Insert(text) => SearchOffer {
                    label: offer.label,
                    chip: None,
                    insert: Some(text),
                },
            })
            .collect())
    }

    /// Takes the finished words of `text` apart: those that are chips
    /// become chips, the others stay in `rest`, and so does the last word
    /// unless a space ends the text, so a chip is made only when its word
    /// is done.
    pub fn search_split(&self, text: String) -> Result<SearchSplit, MobileError> {
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let snapshot = objects.snapshot();
        let cut = text
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_whitespace())
            .map_or(0, |(i, c)| i + c.len_utf8());
        let (done, typing) = text.split_at(cut);
        let (chips, free) = query::split_search(done, &snapshot);
        let rest = [free.trim(), typing]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        // A space after free words stays, so the next word starts anew.
        let rest = if typing.is_empty() && !rest.is_empty() {
            format!("{rest} ")
        } else {
            rest
        };
        Ok(SearchSplit {
            chips: chips
                .iter()
                .map(|c| row(c, &snapshot, self.ui_lang()))
                .collect(),
            rest,
        })
    }

    /// A chip from its parts, with word and label: to negate one, or for
    /// the quick filters. A kind or value the search does not know is an
    /// input error.
    pub fn search_chip(
        &self,
        kind: String,
        value: String,
        not: bool,
    ) -> Result<SearchChip, MobileError> {
        let kind: ScopeChipKind = serde_json::from_value(Value::String(kind.clone()))
            .ok()
            .filter(|k| *k != ScopeChipKind::Unknown)
            .ok_or_else(|| input(format!("no chip kind {kind}")))?;
        let chip = ScopeChip { kind, value, not };
        query::check(&chip).map_err(|e| input(e.to_string()))?;
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        Ok(row(&chip, &objects.snapshot(), self.ui_lang()))
    }

    /// The last searches on this phone, the newest first (ADR 0103 §3).
    pub fn recent_searches(&self) -> Result<Vec<String>, MobileError> {
        let cache = self.cache()?;
        Ok(cache
            .meta(RECENT_KEY)?
            .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
            .unwrap_or_default())
    }

    /// Remembers a search on this phone: first in the list, once, at most
    /// five. An empty text is not remembered.
    pub fn remember_search(&self, text: String) -> Result<(), MobileError> {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            return Ok(());
        }
        let mut list = self.recent_searches()?;
        list.retain(|t| *t != text);
        list.insert(0, text);
        list.truncate(RECENT);
        let json = serde_json::to_string(&list).map_err(|e| input(e.to_string()))?;
        self.cache()?.set_meta(RECENT_KEY, Some(&json))?;
        Ok(())
    }
}
