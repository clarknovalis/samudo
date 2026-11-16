// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later OR MIT OR Apache-2.0

//! The hybrid logical clock stamp (ADR 0015 §4, ADR 0016 §1).
//!
//! The type and its order live here, the arithmetic of the next stamp in
//! samu-core. The order is part of the type: a server and a phone that sorted
//! stamps differently would disagree about which change is the last.

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};

/// A point in hybrid logical time, as in catastr (catastr ADR 0029).
///
/// On the wire `{"wall": 1762502400000, "counter": 3, "device": "pixel"}`.
/// Ordered by wall clock, then counter, then device; the device only breaks
/// a remaining tie, so that the order is total.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Hlc {
    /// Milliseconds since the Unix epoch.
    pub wall: i64,
    /// Separates changes within the same millisecond.
    pub counter: u32,
    /// Which device produced the stamp.
    pub device: String,
}

impl Hlc {
    #[must_use]
    pub fn new(wall: i64, counter: u32, device: impl Into<String>) -> Self {
        Self {
            wall,
            counter,
            device: device.into(),
        }
    }
}

/// Written by hand rather than derived: a derived order follows the field
/// order and would silently change if a field were added above `wall`.
impl Ord for Hlc {
    fn cmp(&self, other: &Self) -> Ordering {
        self.wall
            .cmp(&other.wall)
            .then(self.counter.cmp(&other.counter))
            .then_with(|| self.device.cmp(&other.device))
    }
}

impl PartialOrd for Hlc {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// `1762502400000.3@pixel`, for logs and the command line only.
impl fmt::Display for Hlc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}@{}", self.wall, self.counter, self.device)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_clock_decides_first() {
        assert!(Hlc::new(200, 0, "a") > Hlc::new(100, 9, "z"));
    }

    #[test]
    fn the_counter_separates_the_same_millisecond() {
        assert!(Hlc::new(100, 1, "a") > Hlc::new(100, 0, "z"));
    }

    #[test]
    fn the_device_breaks_a_remaining_tie() {
        assert!(Hlc::new(100, 0, "phone") > Hlc::new(100, 0, "laptop"));
        assert_eq!(
            Hlc::new(100, 0, "x").cmp(&Hlc::new(100, 0, "x")),
            Ordering::Equal
        );
    }

    #[test]
    fn display_is_wall_counter_at_device() {
        assert_eq!(
            Hlc::new(1_762_502_400_000, 3, "pixel").to_string(),
            "1762502400000.3@pixel"
        );
    }
}
