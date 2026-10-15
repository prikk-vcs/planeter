//! The one door from an async handler into the authentication layer (RFC 002 follow-on, IS-24).
//!
//! Everything behind `planeter-auth` is synchronous and blocking: SQLite lookups behind a mutex, token
//! hashing, session reads, Argon2id verification and, for SSO, a confined `curl`. Called straight from an
//! `async fn` each of these pins a runtime worker, and a sign-in flood is reachable without credentials.
//! So handlers never touch `state.auth`, `state.sessions` or `state.oidc` themselves; they come through
//! here, and a source-scan test (`tests/auth_gate.rs`) forbids anything else.
//!
//! Two doors:
//!
//! - [`blocking`]: short, bounded work (a token or session check, a session create/delete, the SSO
//!   hand-shake) on the blocking pool, **no permit**;
//! - [`blocking_gated`]: password verification, which takes a permit from the **auth gate**
//!   (`AppState::auth_gate`, separate from the prikk gate). Every password verification takes one,
//!   *including the dummy verification for an unknown account* (IS-31), so a refusal under load is the
//!   same for existing and unknown names.

use std::sync::Arc;

use axum::http::{HeaderMap, header};
use planeter_auth::{Authenticator, OidcProvider, SessionId, SessionStore};
use planeter_core::Principal;

use crate::api::AppState;
use crate::cookies::{SESSION_COOKIE, get_cookie};
use crate::gate::{GateError, offload};

/// The services a blocking job may use. Borrowed from `Arc`s the job owns.
pub(crate) struct Services<'a> {
    pub auth: &'a Authenticator,
    pub sessions: &'a dyn SessionStore,
    pub oidc: Option<&'a OidcProvider>,
}

fn owned(
    state: &AppState,
) -> (
    Arc<Authenticator>,
    Arc<dyn SessionStore>,
    Option<Arc<OidcProvider>>,
) {
    (
        Arc::clone(&state.auth),
        Arc::clone(&state.sessions),
        state.oidc.clone(),
    )
}

/// Run `job` on the blocking pool without a permit.
pub(crate) async fn blocking<T, F>(state: &AppState, job: F) -> Result<T, GateError>
where
    F: FnOnce(Services<'_>) -> T + Send + 'static,
    T: Send + 'static,
{
    let (auth, sessions, oidc) = owned(state);
    offload(move || {
        job(Services {
            auth: &auth,
            sessions: &*sessions,
            oidc: oidc.as_deref(),
        })
    })
    .await
}

/// Run `job` on the blocking pool under a permit from the auth gate (bounded wait, then
/// [`GateError::Busy`]). For password verification only.
pub(crate) async fn blocking_gated<T, F>(state: &AppState, job: F) -> Result<T, GateError>
where
    F: FnOnce(Services<'_>) -> T + Send + 'static,
    T: Send + 'static,
{
    let (auth, sessions, oidc) = owned(state);
    state
        .auth_gate
        .run(move || {
            job(Services {
                auth: &auth,
                sessions: &*sessions,
                oidc: oidc.as_deref(),
            })
        })
        .await
}

/// The one place a request becomes a [`Principal`]: a bearer token (the API path), else a session cookie
/// (the UI path), else anonymous. A request carrying neither costs nothing; otherwise the lookups run on
/// the blocking pool, without a permit (token and session checks are cheap and take no Argon2id).
pub(crate) async fn principal(headers: &HeaderMap, state: &AppState) -> Principal {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.trim().to_owned());
    let session_id = get_cookie(headers, SESSION_COOKIE);
    if bearer.is_none() && session_id.is_none() {
        return Principal::Anonymous;
    }
    let now = (state.now_unix)();
    let resolved = blocking(state, move |s| {
        if let Some(token) = bearer {
            if let Ok(principal) = s.auth.authenticate_token(&token) {
                return principal;
            }
        }
        if let Some(id) = session_id {
            if let Ok(Some(session)) = s.sessions.get(&SessionId(id), now) {
                return Principal::User(session.user);
            }
        }
        Principal::Anonymous
    })
    .await;
    // A job that did not complete (a panic, a shutdown) authenticates nobody.
    resolved.unwrap_or(Principal::Anonymous)
}
