//! What `GET /events` sends (ADR 0014 §10, catastr ADR 0095).

use serde::{Deserialize, Serialize};

use crate::Id;

/// One `changed` event.
///
/// Ids only, never content: whoever wants a title asks the routes that show
/// titles, with the same rights. An object is in exactly one list; `gone`
/// wins over `added`, `added` over `changed`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeNotice {
    /// The account's change number of the last change in here (ADR 0004 §1).
    pub seq: u64,
    /// Deleted: out of every list.
    #[serde(default)]
    pub gone: Vec<Id>,
    /// New.
    #[serde(default)]
    pub added: Vec<Id>,
    /// Anything else about an object.
    #[serde(default)]
    pub changed: Vec<Id>,
    /// Something without an id changed: the account's settings.
    #[serde(default)]
    pub config: bool,
}

/// The `reset` event: load what you show once more, from scratch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResetNotice {
    /// The change number the listener is now at.
    pub seq: u64,
}
