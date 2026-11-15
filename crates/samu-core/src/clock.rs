//! The arithmetic of the hybrid logical clock and last-writer-wins
//! (ADR 0004 §3/§4, ADR 0016 §1/§4).
//!
//! Rebuilt from catastr's `catastr-core/src/clock.rs` (catastr ADR 0029), not
//! shared: the function is small, and a shared crate would tie the two
//! release cycles together (ADR 0004).

use jiff::Timestamp;

use crate::api::{Change, Hlc};

/// How far ahead of this machine's clock a foreign stamp may be before it is
/// treated as a broken clock rather than a fast one.
///
/// Without a limit, one device a year in the future after a dead battery
/// would pull every other device there, and nothing would sort after it
/// again.
pub const MAX_DRIFT_MS: i64 = 60 * 60 * 1000;

/// The next stamp for a change made on `device`.
///
/// The rule of Kulkarni et al.: the wall part never goes backwards, and it
/// follows the highest wall time this device has *seen*, in stamps from
/// elsewhere too. So a change made after seeing another one sorts after it,
/// whatever the two wall clocks say.
///
/// `own` is the highest stamp this device produced, `seen` the highest it
/// received from any other. Both may be absent. The state stays with the
/// caller (in the database); this function holds none.
#[must_use]
pub fn next_stamp(device: &str, own: Option<&Hlc>, seen: Option<&Hlc>, now: Timestamp) -> Hlc {
    let physical = now.as_millisecond();

    let (own_wall, own_counter) = own.map_or((0, 0), |stamp| (stamp.wall, stamp.counter));
    // A stamp far in the future is a broken clock: capped, not followed.
    let (seen_wall, seen_counter) = seen.map_or((0, 0), |stamp| {
        (
            stamp.wall.min(physical.saturating_add(MAX_DRIFT_MS)),
            stamp.counter,
        )
    });

    let highest = physical.max(own_wall).max(seen_wall);
    // Only a stamp that reaches the highest wall contributes its counter.
    let at_own = highest == own_wall && own.is_some();
    let at_seen = highest == seen_wall && seen.is_some();
    let counter = match (at_own, at_seen) {
        (true, true) => own_counter.max(seen_counter).saturating_add(1),
        (true, false) => own_counter.saturating_add(1),
        (false, true) => seen_counter.saturating_add(1),
        (false, false) => 0,
    };

    Hlc::new(highest, counter, device)
}

/// Whether a change stamped `incoming` replaces what holds now (ADR 0016 §4).
///
/// `current` is the stamp of the last change to the same field or set
/// element; none if it was never set. Only a strictly later stamp wins: the
/// same stamp again is the same change arriving twice.
#[must_use]
pub fn wins(incoming: &Hlc, current: Option<&Hlc>) -> bool {
    current.is_none_or(|current| incoming > current)
}

/// `change` with its stamp replaced by `stamp`: for a writer that stamps
/// on behalf of its client (ADR 0019 §4, ADR 0066 §2).
#[must_use]
pub fn restamp(change: Change, stamp: Hlc) -> Change {
    match change {
        Change::Create {
            object,
            kind,
            value,
            ..
        } => Change::Create {
            object,
            kind,
            value,
            hlc: stamp,
        },
        Change::Set {
            object,
            field,
            value,
            note,
            ..
        } => Change::Set {
            object,
            field,
            value,
            note,
            hlc: stamp,
        },
        Change::Add {
            object,
            field,
            element,
            position,
            note,
            ..
        } => Change::Add {
            object,
            field,
            element,
            position,
            note,
            hlc: stamp,
        },
        Change::Remove {
            object,
            field,
            element,
            note,
            ..
        } => Change::Remove {
            object,
            field,
            element,
            note,
            hlc: stamp,
        },
        Change::Delete { object, .. } => Change::Delete { object, hlc: stamp },
        Change::Unknown => Change::Unknown,
    }
}

#[cfg(test)]
mod tests {
    //! Ported from catastr's `next_stamp_tests`; they check the arithmetic.

    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;

    fn at(millis: i64) -> Timestamp {
        Timestamp::from_millisecond(millis).unwrap()
    }

    /// A device that keeps writing: the previous stamp becomes `own`.
    fn tick(device: &str, own: Option<&Hlc>, millis: i64) -> Hlc {
        next_stamp(device, own, None, at(millis))
    }

    #[test]
    fn a_stamp_follows_the_wall_clock() {
        let first = tick("laptop", None, 1000);
        let second = tick("laptop", Some(&first), 2000);
        assert_eq!(first.wall, 1000);
        assert_eq!(second.wall, 2000);
        assert_eq!(second.counter, 0, "a new millisecond restarts the counter");
    }

    #[test]
    fn two_changes_in_one_millisecond_are_still_ordered() {
        let first = tick("laptop", None, 1000);
        let second = tick("laptop", Some(&first), 1000);
        assert!(second > first);
        assert_eq!((second.wall, second.counter), (1000, 1));
    }

    /// NTP stepping the clock back must not make new changes look older.
    #[test]
    fn a_clock_going_backwards_does_not_reorder() {
        let before = tick("laptop", None, 5000);
        let after = tick("laptop", Some(&before), 1000);
        assert!(after > before);
        assert_eq!(after.wall, 5000, "the wall part never goes backwards");
    }

    /// Causality across devices: the property catastr's store once lacked
    /// because it never passed `seen`.
    #[test]
    fn a_change_after_an_observation_sorts_later() {
        let from_laptop = Hlc::new(60_000, 0, "laptop");
        let observed = next_stamp("phone", None, Some(&from_laptop), at(1000));
        let own = next_stamp("phone", Some(&observed), Some(&from_laptop), at(1001));
        assert!(observed > from_laptop);
        assert!(own > observed);
    }

    #[test]
    fn observing_an_older_stamp_still_moves_forward() {
        let own = tick("laptop", None, 5000);
        let stale = Hlc::new(1000, 0, "phone");
        let next = next_stamp("laptop", Some(&own), Some(&stale), at(5000));
        assert!(next > own);
    }

    #[test]
    fn a_broken_remote_clock_is_capped() {
        let far_future = Hlc::new(1000 + 400 * 24 * 3600 * 1000, 0, "phone");
        let next = next_stamp("laptop", None, Some(&far_future), at(1000));
        assert!(
            next.wall <= 1000 + MAX_DRIFT_MS,
            "followed a year ahead: {}",
            next.wall
        );
    }

    #[test]
    fn a_clock_ahead_within_the_drift_is_followed() {
        let ahead = Hlc::new(1000 + 60_000, 4, "phone");
        let next = next_stamp("laptop", None, Some(&ahead), at(1000));
        assert_eq!((next.wall, next.counter), (61_000, 5));
    }

    #[test]
    fn a_device_that_has_written_nothing_starts_from_the_clock() {
        let first = next_stamp("laptop", None, None, at(1000));
        assert_eq!(first, Hlc::new(1000, 0, "laptop"));
    }

    #[test]
    fn simultaneous_changes_on_two_devices_still_order() {
        let a = tick("laptop", None, 1000);
        let b = tick("phone", None, 1000);
        assert_ne!(a, b);
        assert!(a < b || b < a, "one of them has to be later");
    }

    #[test]
    fn a_later_change_wins() {
        let old = Hlc::new(1000, 0, "laptop");
        let new = Hlc::new(1000, 1, "laptop");
        assert!(wins(&new, Some(&old)));
        assert!(!wins(&old, Some(&new)), "an older change is superseded");
    }

    #[test]
    fn a_field_never_set_takes_any_change() {
        assert!(wins(&Hlc::new(0, 0, "laptop"), None));
    }

    /// The outbox may send the same change twice after a lost answer.
    #[test]
    fn the_same_change_twice_does_not_win_again() {
        let stamp = Hlc::new(1000, 0, "phone");
        assert!(!wins(&stamp, Some(&stamp.clone())));
    }

    /// ADR 0004 §4: on equal time and counter the device decides.
    #[test]
    fn on_a_tie_the_device_decides() {
        let laptop = Hlc::new(1000, 0, "laptop");
        let phone = Hlc::new(1000, 0, "phone");
        assert!(wins(&phone, Some(&laptop)));
        assert!(!wins(&laptop, Some(&phone)));
    }
}
