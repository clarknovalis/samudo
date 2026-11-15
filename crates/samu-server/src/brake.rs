//! The brake on failed sign-ins (ADR 0097 §1): so many failures in a
//! window, and the key waits until the window ends. Only in memory; a
//! restart forgets it.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// How long failures are counted.
pub(crate) const WINDOW: Duration = Duration::from_secs(15 * 60);
/// Failed sign-ins per account name in a window.
pub(crate) const PER_ACCOUNT: u32 = 5;
/// Failed pairing codes in the whole server in a window.
pub(crate) const PAIRING: u32 = 20;
/// Password checks at the same time in the whole server.
pub(crate) const CHECKS_AT_ONCE: usize = 2;

/// More keys than this, and the expired ones go.
const PRUNE_ABOVE: usize = 10_000;

/// Failures in a window, by key.
#[derive(Debug)]
pub(crate) struct Brake {
    limit: u32,
    window: Duration,
    failures: Mutex<HashMap<String, Window>>,
}

#[derive(Debug, Clone, Copy)]
struct Window {
    start: Instant,
    count: u32,
}

impl Brake {
    pub(crate) fn new(limit: u32, window: Duration) -> Self {
        Self {
            limit,
            window,
            failures: Mutex::new(HashMap::new()),
        }
    }

    /// A key as it is counted: lower case, and not longer than an
    /// account name would be.
    pub(crate) fn key(name: &str) -> String {
        name.trim()
            .chars()
            .take(64)
            .collect::<String>()
            .to_lowercase()
    }

    /// How long `key` must still wait; `None` if it may try now.
    pub(crate) fn wait(&self, key: &str, now: Instant) -> Option<Duration> {
        let failures = self.failures.lock().unwrap_or_else(PoisonError::into_inner);
        let window = failures.get(key)?;
        let end = window.start + self.window;
        (window.count >= self.limit && now < end).then(|| end - now)
    }

    /// Counts a failure of `key`.
    pub(crate) fn failed(&self, key: &str, now: Instant) {
        let mut failures = self.failures.lock().unwrap_or_else(PoisonError::into_inner);
        if failures.len() > PRUNE_ABOVE {
            let span = self.window;
            failures.retain(|_, w| now < w.start + span);
        }
        let window = failures.entry(key.to_string()).or_insert(Window {
            start: now,
            count: 0,
        });
        if now >= window.start + self.window {
            *window = Window {
                start: now,
                count: 0,
            };
        }
        window.count = window.count.saturating_add(1);
    }

    /// Forgets the failures of `key`.
    pub(crate) fn succeeded(&self, key: &str) {
        self.failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
    }
}

/// The message of a refused try, rounded up to whole seconds.
pub(crate) fn message(wait: Duration) -> (u64, String) {
    let seconds = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
    (
        seconds,
        format!("too many failed sign-ins, try again in {seconds} s"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limit_holds_a_key_until_the_window_ends() {
        let brake = Brake::new(3, Duration::from_secs(60));
        let start = Instant::now();
        for _ in 0..2 {
            brake.failed("a", start);
            assert_eq!(brake.wait("a", start), None);
        }
        brake.failed("a", start);
        let later = start + Duration::from_secs(20);
        assert_eq!(brake.wait("a", later), Some(Duration::from_secs(40)));
        // Another key is not held.
        assert_eq!(brake.wait("b", later), None);
        // After the window it may try again, and counts from one.
        let after = start + Duration::from_secs(60);
        assert_eq!(brake.wait("a", after), None);
        brake.failed("a", after);
        assert_eq!(brake.wait("a", after), None);
    }

    #[test]
    fn a_success_forgets_the_failures() {
        let brake = Brake::new(2, Duration::from_secs(60));
        let now = Instant::now();
        brake.failed("a", now);
        brake.succeeded("a");
        brake.failed("a", now);
        assert_eq!(brake.wait("a", now), None);
    }

    #[test]
    fn keys_ignore_case_and_length() {
        assert_eq!(Brake::key(" Privat "), "privat");
        assert_eq!(Brake::key(&"x".repeat(500)).len(), 64);
    }

    #[test]
    fn the_message_rounds_up() {
        assert_eq!(message(Duration::from_millis(1500)).0, 2);
        assert_eq!(message(Duration::from_secs(3)).0, 3);
    }
}
