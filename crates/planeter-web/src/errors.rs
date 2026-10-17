//! The read API's one error shape (RFC 003 STD-3): `{ "code", "message", "hint" }`.
//!
//! Every non-2xx, non-304 response of the API carries this body as `application/json`. The codes are
//! stable; the messages and hints are written **once, here** (they will be catalogued for translation
//! later), one plain sentence each, and they never carry a path, a repository's existence or the
//! output of a tool. `not_found` is the same for a hidden repository and an absent one.

use axum::http::StatusCode;

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
    /// 404 from the raw route on any host but the content origin's. Decided from the request's host
    /// alone, before any lookup, so it says nothing about any repository.
    RawElsewhere,
    /// 500: an internal fault.
    Internal,
}

impl ErrorCode {
    /// Every code, in a fixed order (the OpenAPI enum).
    pub const ALL: [ErrorCode; 7] = [
        ErrorCode::NotFound,
        ErrorCode::Unavailable,
        ErrorCode::Busy,
        ErrorCode::InvalidRequest,
        ErrorCode::MethodNotAllowed,
        ErrorCode::RawElsewhere,
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
            ErrorCode::Internal => "internal",
        }
    }

    /// The HTTP status.
    pub fn status(self) -> StatusCode {
        match self {
            ErrorCode::NotFound | ErrorCode::RawElsewhere => StatusCode::NOT_FOUND,
            ErrorCode::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            ErrorCode::Unavailable | ErrorCode::Busy => StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::InvalidRequest => StatusCode::BAD_REQUEST,
            ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// One plain sentence.
    pub fn message(self) -> &'static str {
        match self {
            ErrorCode::NotFound => "No such resource.",
            ErrorCode::Unavailable => "The repository could not be read right now.",
            ErrorCode::Busy => "The server is busy.",
            ErrorCode::InvalidRequest => "The request is malformed.",
            ErrorCode::MethodNotAllowed => "That method is not supported here.",
            ErrorCode::RawElsewhere => "Raw file bytes are not served from this address.",
            ErrorCode::Internal => "The server could not complete the request.",
        }
    }

    /// What the caller can do next, one sentence.
    pub fn hint(self) -> &'static str {
        match self {
            ErrorCode::NotFound => {
                "Check the owner, the repository name and the path, and that your token may read it."
            }
            ErrorCode::Unavailable => "Try again in a little while.",
            ErrorCode::Busy => "Retry after the number of seconds in the Retry-After header.",
            ErrorCode::InvalidRequest => {
                "Check the request against the OpenAPI document at /api/v1/openapi.json."
            }
            ErrorCode::MethodNotAllowed => "Use GET.",
            ErrorCode::RawElsewhere => {
                "Use the link on the file's page, or the content origin named in the OpenAPI document."
            }
            ErrorCode::Internal => "Try again later; if it persists, tell the operator.",
        }
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
            ErrorCode::Internal => "an internal fault",
        }
    }

    /// The JSON body: `{ "code", "message", "hint" }`.
    pub fn body(self) -> String {
        serde_json::json!({
            "code": self.code(),
            "message": self.message(),
            "hint": self.hint(),
        })
        .to_string()
    }
}
