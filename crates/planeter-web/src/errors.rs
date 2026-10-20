//! The read API's one error shape (RFC 003 STD-3): `{ "code", "message", "hint" }`.
//!
//! Every non-2xx, non-304 response of the API carries this body as `application/json`. The codes are
//! stable; the messages and hints are catalog messages ([`crate::catalog`]), one plain sentence each, in
//! the language the caller negotiated (`Accept-Language`, English by default), and they never carry a path,
//! a repository's existence or the output of a tool. `not_found` is the same for a hidden repository and an
//! absent one. The `description` texts are the OpenAPI document's developer-facing prose and stay English.

use axum::http::StatusCode;

use crate::catalog::{EN, Key, Lang};

/// The catalog keys of one error body: its message and its hint. Chosen at the call site, so that one code
/// can have more than one hint (`too_large`) without anyone matching on text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorKeys {
    pub message: Key,
    pub hint: Key,
}

/// What the response layer needs to write an error body in the caller's language: the code and the keys.
/// Attached to the response as an extension by the one place that builds error bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub keys: ErrorKeys,
}

/// A stable, machine-readable error code of the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// 404: no such resource, or the caller may not read it (indistinguishable).
    NotFound,
    /// 503: the repository could not be read right now.
    Unavailable,
    /// 503 from the gate: too many requests in flight; carries `Retry-After`.
    Busy,
    /// 400: a malformed query parameter.
    InvalidRequest,
    /// 405: the method is not supported on this route; carries `Allow: GET`.
    MethodNotAllowed,
    /// 422: the response would exceed what this server serves, or the repository needs more memory than
    /// the read budget permits (RFC 011 D-4). Not retryable as is, unlike `busy`. The hint depends on the
    /// cause ([`ErrorCode::keys`] for a file or the budget, [`ErrorCode::TOO_LARGE_RAW`] for the raw route).
    TooLarge,
    /// 404 from the raw route on any host but the content origin's. Decided from the request's host
    /// alone, before any lookup, so it says nothing about any repository.
    RawElsewhere,
    /// 500: an internal fault.
    Internal,
}

impl ErrorCode {
    /// Every code, in a fixed order (the OpenAPI enum).
    pub const ALL: [ErrorCode; 8] = [
        ErrorCode::NotFound,
        ErrorCode::Unavailable,
        ErrorCode::Busy,
        ErrorCode::InvalidRequest,
        ErrorCode::MethodNotAllowed,
        ErrorCode::RawElsewhere,
        ErrorCode::TooLarge,
        ErrorCode::Internal,
    ];

    /// The stable snake_case code.
    pub fn code(self) -> &'static str {
        match self {
            ErrorCode::NotFound => "not_found",
            ErrorCode::Unavailable => "unavailable",
            ErrorCode::Busy => "busy",
            ErrorCode::InvalidRequest => "invalid_request",
            ErrorCode::MethodNotAllowed => "method_not_allowed",
            ErrorCode::RawElsewhere => "raw_elsewhere",
            ErrorCode::TooLarge => "too_large",
            ErrorCode::Internal => "internal",
        }
    }

    /// The HTTP status.
    pub fn status(self) -> StatusCode {
        match self {
            ErrorCode::NotFound | ErrorCode::RawElsewhere => StatusCode::NOT_FOUND,
            ErrorCode::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            ErrorCode::TooLarge => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::Unavailable | ErrorCode::Busy => StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::InvalidRequest => StatusCode::BAD_REQUEST,
            ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// The catalog keys of this code's message and (default) hint.
    pub fn keys(self) -> ErrorKeys {
        let (message, hint) = match self {
            ErrorCode::NotFound => (Key::ErrNotFoundMessage, Key::ErrNotFoundHint),
            ErrorCode::Unavailable => (Key::ErrUnavailableMessage, Key::ErrUnavailableHint),
            ErrorCode::Busy => (Key::ErrBusyMessage, Key::ErrBusyHint),
            ErrorCode::InvalidRequest => {
                (Key::ErrInvalidRequestMessage, Key::ErrInvalidRequestHint)
            }
            ErrorCode::MethodNotAllowed => (
                Key::ErrMethodNotAllowedMessage,
                Key::ErrMethodNotAllowedHint,
            ),
            ErrorCode::RawElsewhere => (Key::ErrRawElsewhereMessage, Key::ErrRawElsewhereHint),
            ErrorCode::TooLarge => (Key::ErrTooLargeMessage, Key::ErrTooLargeHint),
            ErrorCode::Internal => (Key::ErrInternalMessage, Key::ErrInternalHint),
        };
        ErrorKeys { message, hint }
    }

    /// `too_large` from the raw route: the same code, status and message, and the hint that is true there
    /// (the content origin serves no more than the cap, so "download it from there" would be untrue).
    pub const TOO_LARGE_RAW: ErrorKeys = ErrorKeys {
        message: Key::ErrTooLargeMessage,
        hint: Key::ErrTooLargeRawHint,
    };

    fn text(key: Key, lang: Lang) -> String {
        crate::catalog::T::new(lang)
            .msg(key)
            .with("document", crate::api::paths::OPENAPI)
            .text()
    }

    /// The message, in English. One plain sentence.
    pub fn message(self) -> String {
        Self::text(self.keys().message, EN)
    }

    /// What the caller can do next, in English, one sentence.
    pub fn hint(self) -> String {
        Self::text(self.keys().hint, EN)
    }

    /// A short description for the OpenAPI document.
    pub fn description(self) -> &'static str {
        match self {
            ErrorCode::NotFound => "no such resource, or the caller may not read it",
            ErrorCode::Unavailable => "the repository could not be read right now",
            ErrorCode::Busy => "too many requests in flight (carries Retry-After)",
            ErrorCode::InvalidRequest => "a malformed parameter",
            ErrorCode::MethodNotAllowed => {
                "the method is not supported (the response carries `Allow: GET`)"
            }
            ErrorCode::RawElsewhere => {
                "the raw route, asked on a host other than the content origin's (raw bytes are served only there)"
            }
            ErrorCode::TooLarge => {
                "the response would exceed what this server serves (a file above the file or raw bound), or the repository needs more memory than the read budget permits (not retryable as is; the hint names the remedy for the cause)"
            }
            ErrorCode::Internal => "an internal fault",
        }
    }

    /// The JSON body in English: `{ "code", "message", "hint" }`.
    pub fn body(self) -> String {
        self.body_for(self.keys(), EN)
    }

    /// The JSON body for these keys in `lang`. One code, one status; the message and hint are the
    /// language-dependent fields.
    pub fn body_for(self, keys: ErrorKeys, lang: Lang) -> String {
        serde_json::json!({
            "code": self.code(),
            "message": Self::text(keys.message, lang),
            "hint": Self::text(keys.hint, lang),
        })
        .to_string()
    }
}
