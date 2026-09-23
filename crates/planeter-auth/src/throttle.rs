//! Login throttling (threat model T-2 / **C-2b**: "rate-limited and lockout-protected authentication").
//! Password guessing against `/login` is bounded **per account**: after `max_failures` consecutive
//! failures the account is locked for `lock_secs`, and a correct password during the lock is still
//! refused. A success clears the counter. Per-account (not per-IP) because the client IP is only
//! trustworthy once the trusted-proxy header work lands (TLS is proxy-terminated); per-IP throttling is
//! layered on then. In-memory: bounded, per process (documented residual RR-7).

use std::collections::HashMap;
use std::sync::Mutex;

/// Default policy: 5 consecutive failures → 15 minutes locked.
pub const DEFAULT_MAX_FAILURES: u32 = 5;
pub const DEFAULT_LOCK_SECS: u64 = 15 * 60;
/// Cap on tracked accounts, so a flood of made-up usernames cannot grow memory without bound.
const MAX_TRACKED: usize = 10_000;

#[derive(Debug, Clone, Copy)]
struct Entry {
    failures: u32,
    locked_until: u64,
    last_seen: u64,
}

/// Per-account failure counting with lockout.
pub struct LoginThrottle {
    max_failures: u32,
    lock_secs: u64,
    state: Mutex<HashMap<String, Entry>>,
}

impl Default for LoginThrottle {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_FAILURES, DEFAULT_LOCK_SECS)
    }
}

impl LoginThrottle {
    pub fn new(max_failures: u32, lock_secs: u64) -> Self {
        Self {
            max_failures: max_failures.max(1),
            lock_secs,
            state: Mutex::new(HashMap::new()),
        }
    }

    fn key(user: &str) -> String {
        user.trim().to_ascii_lowercase()
    }

    /// May this account attempt a sign-in now? `Err(locked_until)` while locked.
    pub fn check(&self, user: &str, now_unix: u64) -> Result<(), u64> {
        let Ok(map) = self.state.lock() else {
            return Ok(());
        };
        match map.get(&Self::key(user)) {
            Some(e) if e.locked_until > now_unix => Err(e.locked_until),
            _ => Ok(()),
        }
    }

    /// Record a failed attempt; returns `Some(locked_until)` if this failure locked the account.
    pub fn record_failure(&self, user: &str, now_unix: u64) -> Option<u64> {
        let Ok(mut map) = self.state.lock() else {
            return None;
        };
        if map.len() >= MAX_TRACKED {
            // Drop stale entries first; if still full, drop the oldest-seen.
            map.retain(|_, e| e.locked_until > now_unix || now_unix - e.last_seen < self.lock_secs);
            if map.len() >= MAX_TRACKED
                && let Some(oldest) = map
                    .iter()
                    .min_by_key(|(_, e)| e.last_seen)
                    .map(|(k, _)| k.clone())
            {
                map.remove(&oldest);
            }
        }
        let e = map.entry(Self::key(user)).or_insert(Entry {
            failures: 0,
            locked_until: 0,
            last_seen: now_unix,
        });
        // A failure after a lock expired starts a fresh count.
        if e.locked_until != 0 && e.locked_until <= now_unix {
            e.failures = 0;
            e.locked_until = 0;
        }
        e.failures += 1;
        e.last_seen = now_unix;
        if e.failures >= self.max_failures {
            e.locked_until = now_unix + self.lock_secs;
            e.failures = 0;
            Some(e.locked_until)
        } else {
            None
        }
    }

    /// A successful sign-in clears the account's counter.
    pub fn record_success(&self, user: &str) {
        if let Ok(mut map) = self.state.lock() {
            map.remove(&Self::key(user));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locks_after_max_failures_and_expires() {
        let t = LoginThrottle::new(3, 100);
        assert!(t.check("Alice", 0).is_ok());
        assert_eq!(t.record_failure("alice", 1), None);
        assert_eq!(t.record_failure("ALICE ", 2), None);
        assert_eq!(t.record_failure("alice", 3), Some(103)); // third failure locks
        assert_eq!(t.check("alice", 50), Err(103));
        assert!(t.check("alice", 103).is_ok()); // lock expired
        // A fresh failure after expiry starts over (not an immediate re-lock).
        assert_eq!(t.record_failure("alice", 104), None);
        // Other accounts are unaffected.
        assert!(t.check("bob", 50).is_ok());
    }

    #[test]
    fn success_clears_the_counter() {
        let t = LoginThrottle::new(3, 100);
        t.record_failure("carol", 1);
        t.record_failure("carol", 2);
        t.record_success("carol");
        assert_eq!(t.record_failure("carol", 3), None); // count restarted at 1
    }

    #[test]
    fn tracked_accounts_are_bounded() {
        let t = LoginThrottle::new(2, 10);
        for i in 0..(MAX_TRACKED + 50) {
            t.record_failure(&format!("u{i}"), 1);
        }
        assert!(t.state.lock().unwrap().len() <= MAX_TRACKED);
    }
}
