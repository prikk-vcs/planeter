//! The one place a request becomes a [`Principal`]: an `Authorization: Bearer` token (the API path) or
//! a browser session cookie (the UI path), else anonymous. Both the API and the UI call this, so there
//! is a single authentication entry point in front of the single `authorize()` decision.

use axum::http::{HeaderMap, header};
use planeter_core::Principal;

use crate::api::AppState;
use crate::cookies::{SESSION_COOKIE, get_cookie};

pub fn principal_from_request(headers: &HeaderMap, state: &AppState) -> Principal {
    if let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        && let Some(token) = value.strip_prefix("Bearer ")
        && let Ok(principal) = state.auth.authenticate_token(token.trim())
    {
        return principal;
    }
    if let Some(id) = get_cookie(headers, SESSION_COOKIE)
        && let Ok(Some(session)) = state
            .sessions
            .get(&planeter_auth::SessionId(id), (state.now_unix)())
    {
        return Principal::User(session.user);
    }
    Principal::Anonymous
}
