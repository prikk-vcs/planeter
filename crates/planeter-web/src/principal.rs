//! The one place a request becomes a [`Principal`]: an `Authorization: Bearer` token (the API path) or
//! a browser session cookie (the UI path), else anonymous. Both the API and the UI call this, so there
//! is a single authentication entry point in front of the single `authorize()` decision.
//!
//! The lookups are blocking (SQLite, token hashing), so they run on the blocking pool through
//! the crate's `authn` module and this function is `async` (IS-24).

use axum::http::HeaderMap;
use planeter_core::Principal;

use crate::api::AppState;

pub async fn principal_from_request(headers: &HeaderMap, state: &AppState) -> Principal {
    crate::authn::principal(headers, state).await
}
