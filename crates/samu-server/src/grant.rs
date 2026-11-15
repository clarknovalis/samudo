//! Restricted sessions on the server (ADR 0095 §4): what each route does
//! for a session with a grant, and the check every change passes. The rule
//! itself is `samu_core::grant`.

use samu_store::AccountStore;
use samu_store::core::api::{AnyObject, Change, ErrorCode, Grant, Route};
use samu_store::core::grant::Reach;
use samu_store::core::overlay::{self, Objects};

use crate::app::Failure;

/// What a route does for a restricted session (ADR 0095 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rule {
    /// Answers with what the grant reaches, and writes only there.
    Filtered,
    /// 403 `forbidden`, whatever is asked.
    Forbidden,
    /// As for any session.
    Free,
}

/// Every route of `samu_api::ROUTES` and its rule. A new route without an
/// entry fails a test (ADR 0095 §4).
pub(crate) fn restriction(route: &Route) -> Option<Rule> {
    use Rule::{Filtered, Forbidden, Free};
    Some(match (route.method, route.path) {
        ("GET" | "POST", "/changes")
        | ("GET", "/objects/{id}")
        | ("GET", "/objects/{id}/logbook")
        | ("POST", "/objects/{id}/delete")
        | ("GET", "/available")
        | ("GET", "/available/projects")
        | ("GET", "/available/folders")
        | ("GET", "/forecast")
        | ("GET", "/clocks/report")
        | ("GET", "/report")
        | ("GET", "/habits")
        | ("GET", "/habits/{id}")
        | ("GET", "/events")
        | ("POST", "/capture")
        | ("PUT" | "GET", "/blobs/{sha256}") => Filtered,
        ("GET", "/scopes/scheduled")
        | ("GET", "/export")
        | ("GET" | "POST", "/hooks")
        | ("POST", "/pair/code")
        | ("GET", "/appointments")
        | ("GET", "/alarms")
        | ("POST", "/appointments/{id}/edit")
        | ("PUT", "/calendars/{id}/source")
        | ("POST", "/calendars/{id}/refresh")
        | ("POST", "/calendars/discover")
        | ("POST", "/calendars/{id}/discover")
        | ("POST", "/calendars/{id}/import")
        | ("GET", "/sessions")
        | ("POST", "/sessions/{id}/end")
        | ("POST", "/account/delete")
        | ("POST", "/demo") => Forbidden,
        // Without a session, or what every session may do.
        ("GET", "/health")
        | ("POST", "/login")
        | ("POST", "/pair")
        | ("GET" | "POST", "/register")
        | ("POST", "/hooks/{name}")
        | ("GET", "/session")
        | ("POST", "/logout")
        | ("POST", "/render") => Free,
        _ => return None,
    })
}

/// The answer for what a restricted session may not do.
pub(crate) fn forbidden(message: impl Into<String>) -> Failure {
    Failure::status(
        axum::http::StatusCode::FORBIDDEN,
        ErrorCode::Forbidden,
        message,
    )
}

/// Why a change is refused to a restricted session.
pub(crate) const OUTSIDE: &str = "outside the session's grant (ADR 0095)";

/// The grant read against the account as it is now.
pub(crate) fn reach(store: &AccountStore, grant: &Grant) -> Result<Reach, Failure> {
    let objects = store.snapshot()?;
    Ok(Reach::new(
        grant,
        &objects.folders,
        &objects.projects,
        &objects.tasks,
    ))
}

/// Whether a change may be applied through the reach (ADR 0095 §3): the
/// object as it is and as the change leaves it. A change the overlay
/// cannot lay over an object the session sees is refused like one outside.
pub(crate) fn may_apply(
    reach: &Reach,
    store: &AccountStore,
    change: &Change,
) -> Result<bool, Failure> {
    let Some(id) = object_of(change) else {
        return Ok(false);
    };
    let before = store.get(id)?.map(AnyObject::from);
    let after = match change {
        // A delete asks with the object on both sides (`may_write`).
        Change::Delete { .. } => before.clone(),
        _ => {
            let mut objects = Objects::from_any(before.clone());
            if overlay::apply(&mut objects, change).is_err() {
                return Ok(false);
            }
            objects.get(id)
        }
    };
    Ok(after.is_some_and(|after| reach.may_write(before.as_ref(), &after)))
}

/// Takes in what a change made or moved, as it is now stored.
pub(crate) fn admit(
    reach: &mut Reach,
    store: &AccountStore,
    change: &Change,
) -> Result<(), Failure> {
    if let Some(id) = object_of(change)
        && let Some(object) = store.get(id)?
    {
        reach.admit(&AnyObject::from(object));
    }
    Ok(())
}

pub(crate) fn object_of(change: &Change) -> Option<samu_store::core::api::Id> {
    match change {
        Change::Create { object, .. }
        | Change::Set { object, .. }
        | Change::Add { object, .. }
        | Change::Remove { object, .. }
        | Change::Delete { object, .. } => Some(*object),
        Change::Unknown => None,
    }
}

/// Whether a session sees an object by id, as it is stored now; a
/// tombstone or an unknown id is not seen.
pub(crate) fn sees_stored(
    reach: &Reach,
    store: &AccountStore,
    id: samu_store::core::api::Id,
) -> Result<bool, Failure> {
    Ok(store
        .get(id)?
        .is_some_and(|object| reach.sees(&AnyObject::from(object))))
}

#[cfg(test)]
mod tests {
    use samu_store::core::api::ROUTES;

    use super::*;

    /// Every route has a rule (ADR 0095 §4), like the replica's table.
    #[test]
    fn every_route_has_a_rule() {
        for route in ROUTES {
            assert!(
                restriction(route).is_some(),
                "{} {} has no rule for a restricted session",
                route.method,
                route.path
            );
        }
    }
}
