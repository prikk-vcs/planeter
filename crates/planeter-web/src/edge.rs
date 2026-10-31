//! The edge (japanese-final-handoff T2): what the framework would answer by itself is answered by the project instead, in one place.
//!
//! Three things reached a person in the framework's own words, or in none: a path or query the framework could not decode (a plain-text
//! `400 Invalid URL: …`), a request body over what it reads for a form or a JSON document (a plain-text `413`, or a handler's
//! "incomplete form" page with a `422`), and an address no route serves or a method a page route does not take (an empty `404` or
//! `405`). [`framework_rejections`] is a layer around the whole router that turns each into the project's own answer: under `/api/` the
//! one JSON shape `{code, message, hint}` (`invalid_request` 400, `payload_too_large` 413), anywhere else a page with a heading, one
//! sentence and one link, in the caller's language, with the same status. No handler is touched.
//!
//! These answers are route-level: they say nothing about whether a repository exists.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::task::{Context, Poll};

use axum::body::{Body, BodyDataStream, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use futures_core::Stream;

use crate::api::{self, AppState};
use crate::errors::ErrorCode;
use crate::ui::{self, EdgePage};

/// What the framework reads for one form or JSON document: its own default (2 MiB), named so that the router sets exactly this and the
/// edge answers exactly this.
pub const BODY_LIMIT: usize = 2 * 1024 * 1024;

/// A request whose body is bounded by its own handler (streamed to disk under its own limits): the edge neither limits nor counts it.
/// The proposal route, the staging of an archive over the API, and the page that stages one.
pub fn has_own_body_limit(method: &Method, path: &str) -> bool {
    *method == Method::POST
        && (path.ends_with("/transport/propose")
            || path == "/api/v1/repos/import"
            || path == "/new/import")
}

fn carries_a_body(method: &Method) -> bool {
    matches!(*method, Method::POST | Method::PUT | Method::PATCH)
}

/// A body that counts what passes and says so, once, if it is more than [`BODY_LIMIT`].
struct Counting {
    inner: BodyDataStream,
    seen: AtomicU64,
    exceeded: Arc<AtomicBool>,
}

impl Stream for Counting {
    type Item = Result<Bytes, axum::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let polled = Pin::new(&mut self.inner).poll_next(cx);
        if let Poll::Ready(Some(Ok(chunk))) = &polled {
            let total =
                self.seen.fetch_add(chunk.len() as u64, Ordering::Relaxed) + chunk.len() as u64;
            if total > BODY_LIMIT as u64 {
                self.exceeded.store(true, Ordering::Relaxed);
            }
        }
        polled
    }
}

fn too_large(state: &AppState, path: &str) -> Option<Response> {
    path.starts_with("/api/").then(|| {
        api::code_response_with_keys(
            state,
            ErrorCode::PayloadTooLarge,
            ErrorCode::TOO_LARGE_REQUEST,
        )
    })
}

async fn answer(state: &AppState, headers: &HeaderMap, path: &str, which: EdgePage) -> Response {
    ui::edge_page(state, headers, path, which).await
}

async fn payload_too_large(state: &AppState, headers: &HeaderMap, path: &str) -> Response {
    match too_large(state, path) {
        Some(r) => r,
        None => answer(state, headers, path, EdgePage::PayloadTooLarge).await,
    }
}

/// A response the framework wrote by itself: a plain-text client error with none of the project's markers.
fn is_framework_rejection(resp: &Response) -> bool {
    matches!(resp.status().as_u16(), 400 | 413 | 415 | 422)
        && resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/plain"))
        && resp
            .extensions()
            .get::<crate::errors::ErrorBody>()
            .is_none()
}

pub async fn framework_rejections(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let (method, path, headers) = (
        req.method().clone(),
        req.uri().path().to_owned(),
        req.headers().clone(),
    );
    let limited = carries_a_body(&method) && !has_own_body_limit(&method, &path);
    let mut exceeded = None;
    let req = if limited {
        // Declared too large: answered before a byte is read.
        let declared = headers
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        if declared.is_some_and(|n| n > BODY_LIMIT as u64) {
            return payload_too_large(&state, &headers, &path).await;
        }
        // Not declared (chunked): counted as it is read.
        let flag = Arc::new(AtomicBool::new(false));
        exceeded = Some(flag.clone());
        let (parts, body) = req.into_parts();
        let counted = Counting {
            inner: body.into_data_stream(),
            seen: AtomicU64::new(0),
            exceeded: flag,
        };
        Request::from_parts(parts, Body::from_stream(counted))
    } else {
        req
    };
    let resp = next.run(req).await;
    if exceeded.is_some_and(|f| f.load(Ordering::Relaxed)) && !resp.status().is_success() {
        return payload_too_large(&state, &headers, &path).await;
    }
    if is_framework_rejection(&resp) {
        if resp.status() == StatusCode::PAYLOAD_TOO_LARGE {
            return payload_too_large(&state, &headers, &path).await;
        }
        return if path.starts_with("/api/") {
            api::code_response(&state, ErrorCode::InvalidRequest)
        } else {
            // An address the framework could not decode names nothing that exists.
            answer(&state, &headers, &path, EdgePage::NotFound).await
        };
    }
    resp
}
