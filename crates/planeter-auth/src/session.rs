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

/// Session lifetime: 7 days.
pub const SESSION_TTL_SECS: u64 = 7 * 24 * 3600;

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
}

/// In-memory [`SessionStore`] for tests/dev.
#[derive(Default)]
pub struct InMemorySessionStore {
    sessions: RwLock<HashMap<String, Session>>,
}

impl InMemorySessionStore {
    pub fn new() -> Self {
        Self::default()
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
    fn csrf_tokens_compare_in_constant_time_and_differ() {
        let a = random_token();
        let b = random_token();
        assert_ne!(a, b);
        assert!(tokens_match(&a, &a));
        assert!(!tokens_match(&a, &b));
        assert!(!tokens_match(&a, &a[..10]));
    }
}
