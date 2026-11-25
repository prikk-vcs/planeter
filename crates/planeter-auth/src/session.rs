//! Browser sessions (RFC 002 T2 / WEB-05 `STD-6`): a signed-in principal is carried by an **opaque
//! random session id** in an `HttpOnly`, `SameSite=Strict` cookie; the server keeps the mapping in a
//! [`SessionStore`]. Nothing about the user is in the cookie itself. Sessions expire.
//!
//! CSRF: forms carry a **double-submit token** — a random value set in a `SameSite=Strict` cookie and
//! echoed in a hidden field; the handler refuses a POST whose field does not equal the cookie. With
//! `SameSite=Strict` this is belt-and-braces, which is the point.

use std::collections::HashMap;
use std::sync::RwLock;

use argon2::password_hash::rand_core::{OsRng, RngCore as _};
use base64ct::{Base64UrlUnpadded, Encoding as _};
use planeter_store::UserId;
use subtle::ConstantTimeEq as _;

use crate::identity::{AuthError, Result};

/// The session lifetime in hours when `PLANETER_SESSION_HOURS` is not set: 7 days (RFC 014 D-13).
pub const SESSION_HOURS_DEFAULT: u64 = 168;
/// The shortest and the longest lifetime the operator may set, in hours.
pub const SESSION_HOURS_MIN: u64 = 1;
pub const SESSION_HOURS_MAX: u64 = 720;
/// The default session lifetime in seconds (**the value of [`SESSION_HOURS_DEFAULT`]; 7 days as it has always been**). A session store
/// reports the lifetime it was set up with ([`SessionStore::ttl_secs`]); this is what a store has until the operator sets another.
pub const SESSION_TTL_SECS: u64 = SESSION_HOURS_DEFAULT * 3600;

/// `PLANETER_SESSION_HOURS` as the number of hours: unset or empty is the default; anything but a whole number from 1 to 720 is **refused,
/// never clamped** (the sentence names the value it got).
pub fn session_hours_from(value: Option<&str>) -> std::result::Result<u64, String> {
    match value.map(str::trim) {
        None | Some("") => Ok(SESSION_HOURS_DEFAULT),
        Some(v) => v
            .parse::<u64>()
            .ok()
            .filter(|h| (SESSION_HOURS_MIN..=SESSION_HOURS_MAX).contains(h))
            .ok_or_else(|| v.to_owned()),
    }
}

/// An opaque session id (32 OS-random bytes, base64url).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);

/// Mint a random opaque token (used for session ids and CSRF tokens).
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    Base64UrlUnpadded::encode_string(&bytes)
}

/// Constant-time equality for CSRF double-submit comparison.
pub fn tokens_match(a: &str, b: &str) -> bool {
    a.len() == b.len() && bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}

/// A live session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: SessionId,
    pub user: UserId,
    pub created_at: u64,
    pub expires_at: u64,
}

/// Persistence of sessions (in-memory now; SQLite in [`crate::sqlite`]).
pub trait SessionStore: Send + Sync {
    /// Create a session for `user` valid until `expires_at`.
    fn create(&self, user: UserId, now_unix: u64, expires_at: u64) -> Result<Session>;
    /// Look up a session; `None` if unknown **or expired** at `now_unix`.
    fn get(&self, id: &SessionId, now_unix: u64) -> Result<Option<Session>>;
    /// Destroy a session (logout).
    fn delete(&self, id: &SessionId) -> Result<()>;
    /// **The lifetime a new session is given, in seconds: the one value, from the one place** (RFC 014 D-13). Both sign-in paths use it for
    /// the session's expiry and for the cookie's `Max-Age`, so they cannot differ. A session that exists keeps the lifetime it was created
    /// with (its `expires_at` is stored).
    fn ttl_secs(&self) -> u64 {
        SESSION_TTL_SECS
    }
}

/// In-memory [`SessionStore`] for tests/dev.
#[derive(Default)]
pub struct InMemorySessionStore {
    sessions: RwLock<HashMap<String, Session>>,
    ttl: Option<u64>,
}

impl InMemorySessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// New sessions last `ttl_secs` seconds.
    #[must_use]
    pub fn with_ttl_secs(mut self, ttl_secs: u64) -> Self {
        self.ttl = Some(ttl_secs);
        self
    }
}

impl SessionStore for InMemorySessionStore {
    fn create(&self, user: UserId, now_unix: u64, expires_at: u64) -> Result<Session> {
        let s = Session {
            id: SessionId(random_token()),
            user,
            created_at: now_unix,
            expires_at,
        };
        self.sessions
            .write()
            .map_err(|_| AuthError::Backend("session lock poisoned".into()))?
            .insert(s.id.0.clone(), s.clone());
        Ok(s)
    }

    fn get(&self, id: &SessionId, now_unix: u64) -> Result<Option<Session>> {
        let g = self
            .sessions
            .read()
            .map_err(|_| AuthError::Backend("session lock poisoned".into()))?;
        Ok(g.get(&id.0).filter(|s| s.expires_at > now_unix).cloned())
    }

    fn delete(&self, id: &SessionId) -> Result<()> {
        self.sessions
            .write()
            .map_err(|_| AuthError::Backend("session lock poisoned".into()))?
            .remove(&id.0);
        Ok(())
    }

    fn ttl_secs(&self) -> u64 {
        self.ttl.unwrap_or(SESSION_TTL_SECS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_are_opaque_expire_and_delete() {
        let s = InMemorySessionStore::new();
        let sess = s
            .create(UserId::new("alice"), 100, 100 + SESSION_TTL_SECS)
            .unwrap();
        assert!(!sess.id.0.contains("alice"));
        assert!(sess.id.0.len() >= 40);
        assert_eq!(
            s.get(&sess.id, 200).unwrap().unwrap().user,
            UserId::new("alice")
        );
        assert!(s.get(&sess.id, 100 + SESSION_TTL_SECS).unwrap().is_none());
        s.delete(&sess.id).unwrap();
        assert!(s.get(&sess.id, 200).unwrap().is_none());
        assert!(s.get(&SessionId("nope".into()), 200).unwrap().is_none());
    }

    #[test]
    fn the_default_lifetime_is_seven_days_in_hours_and_in_seconds() {
        // Fails if one of the two numbers is changed without the other (and without the owner's say).
        assert_eq!(SESSION_HOURS_DEFAULT, 168);
        assert_eq!(SESSION_TTL_SECS, 7 * 24 * 3600);
        assert_eq!(SESSION_TTL_SECS, SESSION_HOURS_DEFAULT * 3600);
        assert_eq!(InMemorySessionStore::new().ttl_secs(), 7 * 24 * 3600);
    }

    #[test]
    fn session_hours_accept_one_to_720_and_refuse_everything_else() {
        assert_eq!(session_hours_from(None), Ok(168));
        assert_eq!(session_hours_from(Some("")), Ok(168));
        assert_eq!(session_hours_from(Some(" 24 ")), Ok(24));
        assert_eq!(session_hours_from(Some("1")), Ok(1));
        assert_eq!(session_hours_from(Some("720")), Ok(720));
        for bad in [
            "0",
            "721",
            "-5",
            "1.5",
            "a day",
            "24h",
            "99999999999999999999",
        ] {
            assert_eq!(session_hours_from(Some(bad)), Err(bad.to_owned()), "{bad}");
        }
    }

    #[test]
    fn csrf_tokens_compare_in_constant_time_and_differ() {
        let a = random_token();
        let b = random_token();
        assert_ne!(a, b);
        assert!(tokens_match(&a, &a));
        assert!(!tokens_match(&a, &b));
        assert!(!tokens_match(&a, &a[..10]));
    }
}
