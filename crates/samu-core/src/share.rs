//! What a text shared to samu makes (ADR 0013): a title, a note and the
//! addresses in it as links. The quick-capture parser does not run on it;
//! only what one types oneself is parsed (ADR 0013 §2, ADR 0010 §8).

use crate::hook::ring_title;

/// The title when the text has no words and there is no subject.
pub const SHARED_WITHOUT_TEXT: &str = "Geteilt";

/// The schemes whose addresses become links of kind `ref`.
const SCHEMES: &[&str] = &["catastr:", "mid:", "https://"];

/// A shared text, taken apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shared {
    pub title: String,
    pub note: Option<String>,
    /// Each address once, in the order it first appears.
    pub links: Vec<String>,
}

/// Title from the subject, else from the first line as the ring cuts it
/// (ADR 0010 §3); the rest of the text as the note, the whole text when
/// the title had to be cut, so nothing is lost.
#[must_use]
pub fn shared(subject: Option<&str>, text: &str) -> Shared {
    let subject = subject.map(str::trim).filter(|s| !s.is_empty());
    let text = text.trim();
    let (title, note) = match subject {
        Some(subject) => (subject.to_string(), text.to_string()),
        None => match ring_title(text) {
            None => (SHARED_WITHOUT_TEXT.to_string(), String::new()),
            Some(title) => {
                let mut lines = text.lines();
                let first = lines.by_ref().find(|l| !l.trim().is_empty()).unwrap_or("");
                let whole_first = first.split_whitespace().collect::<Vec<_>>().join(" ");
                if title == whole_first {
                    (
                        title,
                        lines.collect::<Vec<_>>().join("\n").trim().to_string(),
                    )
                } else {
                    (title, text.to_string())
                }
            }
        },
    };
    let mut links = Vec::new();
    for source in subject.into_iter().chain([text]) {
        for address in addresses(source) {
            if !links.contains(&address) {
                links.push(address);
            }
        }
    }
    Shared {
        title,
        note: (!note.is_empty()).then_some(note),
        links,
    }
}

/// `catastr:`, `mid:` and `https://` addresses in a text, also inside Org
/// links (`[[catastr:…][label]]`), without punctuation at their end.
fn addresses(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || "[]<>()\"".contains(c))
        .map(|word| word.trim_end_matches(['.', ',', ';', ':', '!', '?', '\'']))
        .filter(|word| {
            SCHEMES
                .iter()
                .any(|scheme| word.len() > scheme.len() && word.starts_with(scheme))
        })
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests may unwrap")]
mod tests {
    use super::*;

    #[test]
    fn the_subject_is_the_title_and_the_text_the_note() {
        let s = shared(Some("  Angebot Dachdecker "), "Bitte bis Freitag.\n");
        assert_eq!(s.title, "Angebot Dachdecker");
        assert_eq!(s.note.as_deref(), Some("Bitte bis Freitag."));
        assert!(s.links.is_empty());
    }

    #[test]
    fn without_a_subject_the_first_line_is_the_title() {
        let s = shared(None, "\n  Milch   kaufen \nbeim Bioladen\n\nund Brot");
        assert_eq!(s.title, "Milch kaufen");
        assert_eq!(s.note.as_deref(), Some("beim Bioladen\n\nund Brot"));
        let only = shared(Some(""), "Milch kaufen");
        assert_eq!(only.title, "Milch kaufen");
        assert_eq!(only.note, None);
    }

    #[test]
    fn a_cut_title_keeps_the_whole_text_in_the_note() {
        let line = "wort ".repeat(30);
        let s = shared(None, &format!("{line}\nzweite Zeile"));
        assert!(s.title.ends_with('…') && s.title.chars().count() <= 80);
        let note = s.note.unwrap();
        assert!(note.starts_with("wort wort") && note.ends_with("zweite Zeile"));
    }

    #[test]
    fn nothing_but_space_is_shared_without_text() {
        let s = shared(None, " \n\t ");
        assert_eq!(s.title, SHARED_WITHOUT_TEXT);
        assert_eq!(s.note, None);
    }

    #[test]
    fn addresses_become_links_once_and_in_order() {
        let text = "Aufnahme\ncatastr:01936f5e-8a00-7c3e-9b1a-3f2d4c5b6f00\n\
                    siehe [[catastr:01936f5e-8a00-7c3e-9b1a-3f2d4c5b6f01][Angebot]], \
                    (https://example.org/a?b=1). mid:abc@example.org; \
                    http://nicht.example.org https:// catastr:01936f5e-8a00-7c3e-9b1a-3f2d4c5b6f00";
        let s = shared(None, text);
        assert_eq!(
            s.links,
            [
                "catastr:01936f5e-8a00-7c3e-9b1a-3f2d4c5b6f00",
                "catastr:01936f5e-8a00-7c3e-9b1a-3f2d4c5b6f01",
                "https://example.org/a?b=1",
                "mid:abc@example.org",
            ]
        );
    }

    #[test]
    fn an_address_in_the_subject_counts_too() {
        let s = shared(Some("https://example.org/x"), "");
        assert_eq!(s.links, ["https://example.org/x"]);
    }
}
