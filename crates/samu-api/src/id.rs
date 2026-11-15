//! Object identity (ADR 0014 §2).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The stable id of a samu object: a UUIDv7, on the wire the bare UUID.
///
/// The form `samu:<uuid>` (ADR 0001) is a URI for links; see [`Id::uri`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Id(Uuid);

impl Id {
    /// A new id, from the clock and randomness.
    ///
    /// Clients create ids too, so that creating an object in the outbox is
    /// idempotent (ADR 0004 §8).
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Wraps an existing UUID.
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// The UUID inside.
    #[must_use]
    pub const fn uuid(&self) -> Uuid {
        self.0
    }

    /// `samu:<uuid>`, the form a link from elsewhere points at.
    #[must_use]
    pub fn uri(&self) -> String {
        format!("{}:{}", crate::URI_SCHEME, self.0)
    }

    /// Reads `samu:<uuid>` back. Anything else is `None`.
    #[must_use]
    pub fn from_uri(uri: &str) -> Option<Self> {
        let rest = uri.strip_prefix(crate::URI_SCHEME)?.strip_prefix(':')?;
        rest.parse().ok()
    }
}

impl Default for Id {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for Id {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;

    #[test]
    fn uri_round_trips() {
        let id = Id::new();
        let uri = id.uri();
        assert!(uri.starts_with("samu:"));
        assert_eq!(Id::from_uri(&uri), Some(id));
    }

    #[test]
    fn other_schemes_are_not_ids() {
        let id = Id::new();
        assert_eq!(Id::from_uri(&format!("catastr:{id}")), None);
        assert_eq!(Id::from_uri(&id.to_string()), None);
        assert_eq!(Id::from_uri("samu:not-a-uuid"), None);
    }

    #[test]
    fn new_ids_are_version_7() {
        assert_eq!(Id::new().uuid().get_version_num(), 7);
    }

    #[test]
    fn on_the_wire_it_is_the_bare_uuid() {
        let id: Id = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6a70".parse().unwrap();
        assert_eq!(
            serde_json::to_string(&id).unwrap(),
            "\"01936f5e-8a00-7c3e-9b1a-3f2d4c5b6a70\""
        );
    }
}
