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

/// What an error body carries beyond its code and its two sentences: the values the sentences name (`params`, filled
/// into the catalog message by name) and extra machine-readable `fields`. Attached to the response next to
/// [`ErrorBody`], so the response layer can write the body again in the caller's language.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ErrorDetail {
    pub params: Vec<(&'static str, String)>,
    pub fields: Vec<(&'static str, serde_json::Value)>,
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
    /// 503: artifact fetches are not set up on this server (no export spool configured).
    TransportOff,
    /// 422: the repository has no ref of that name (transport fetch).
    UnknownRef,
    /// 422: the fetch `kind` is neither `archive` nor `bundle`.
    UnknownKind,
    /// 422: a bundle of a received ref was asked for; the archive carries it (PK-60).
    BundleOfReceivedRef,
    /// 503: the exported artifact did not verify and was not served.
    NotVerified,
    /// 503: the export failed, hit its deadline or its memory ceiling, or the artifact exceeds the spool.
    ExportFailed,
    /// 429: the per-principal or per-IP rate limit; carries `Retry-After`.
    RateLimited,
    /// 416: a `Range` that cannot be satisfied by the one current artifact; carries `Content-Range: bytes */size`.
    RangeNotSatisfiable,
    /// 403: the caller can read the repository but may not propose to it.
    Forbidden,
    /// 411: a proposal must declare its length (`Content-Length`).
    LengthRequired,
    /// 413: the proposal is larger than this server accepts; refused before the artifact is read.
    PayloadTooLarge,
    /// 503: the repository is waiting for recovery after an unfinished write; reads are not affected.
    NeedsRecovery,
    /// 422: a limit on proposals was reached (the body names which one, its limit and the current use).
    QuotaExceeded,
    /// 422: the declared base is not a block this repository holds.
    BaseUnknown,
    /// 422: the declared patches leave out patches the artifact added.
    DeclarationMissesAdded,
    /// 422: the declared patches name patches the repository does not hold.
    DeclaredNotHeld,
    /// 422: the artifact added nothing the repository did not already hold.
    NoNewContent,
    /// 422: the repository has no proposal with that id.
    UnknownProposal,
    /// 422: the proposal is landed or closed and takes no more revisions.
    ProposalNotOpen,
    /// 403: the proposal was opened by somebody else, and the caller does not hold `write`.
    NotYourProposal,
    /// 422: prikk refused the artifact and left nothing behind (its text is in the `detail` field).
    AcceptRefused,
    /// 409: somebody else has begun sending this exact artifact and it has not finished.
    IntentPending,
    /// 422: an intent to abandon is not an open intent of this repository.
    UnknownIntent,
    /// 503: the accept did not finish; the body names the intent, and the same artifact can be sent again once the repository has recovered.
    Incomplete,
    /// 422: a proposal is closed with a reason and none was given.
    ReasonRequired,
    /// 422: the proposal has landed or been closed (the body's `state` says which), and a final proposal is closed no more.
    ProposalFinal,
    /// 410: the files of that revision were kept until a date and have been removed since (the body's `kept_until` is it).
    ArtifactRemoved,
    /// 422: the proposal finished more than its grace ago, and takes no more comments (`state`, `kept_until`).
    CommentsClosed,
    /// 422: the proposal has landed or been closed, and takes no more decisions (`state`).
    DecisionsClosed,
    /// 422: a decision was named for a revision that is not the latest (`current`).
    RevisionNotCurrent,
    /// 422: the submitter of a revision (or of any revision of the proposal) cannot approve it (`reason`).
    SelfApproval,
    /// 422: the comment is already hidden, or is not hidden (`hidden` says which it is now).
    CommentState,
}

impl ErrorCode {
    /// Every code, in a fixed order (the OpenAPI enum).
    pub const ALL: [ErrorCode; 40] = [
        ErrorCode::NotFound,
        ErrorCode::Unavailable,
        ErrorCode::Busy,
        ErrorCode::InvalidRequest,
        ErrorCode::MethodNotAllowed,
        ErrorCode::RawElsewhere,
        ErrorCode::TooLarge,
        ErrorCode::Internal,
        ErrorCode::TransportOff,
        ErrorCode::UnknownRef,
        ErrorCode::UnknownKind,
        ErrorCode::BundleOfReceivedRef,
        ErrorCode::NotVerified,
        ErrorCode::ExportFailed,
        ErrorCode::RateLimited,
        ErrorCode::RangeNotSatisfiable,
        ErrorCode::Forbidden,
        ErrorCode::LengthRequired,
        ErrorCode::PayloadTooLarge,
        ErrorCode::NeedsRecovery,
        ErrorCode::QuotaExceeded,
        ErrorCode::BaseUnknown,
        ErrorCode::DeclarationMissesAdded,
        ErrorCode::DeclaredNotHeld,
        ErrorCode::NoNewContent,
        ErrorCode::UnknownProposal,
        ErrorCode::ProposalNotOpen,
        ErrorCode::NotYourProposal,
        ErrorCode::AcceptRefused,
        ErrorCode::IntentPending,
        ErrorCode::Incomplete,
        ErrorCode::UnknownIntent,
        ErrorCode::ReasonRequired,
        ErrorCode::ProposalFinal,
        ErrorCode::ArtifactRemoved,
        ErrorCode::CommentsClosed,
        ErrorCode::DecisionsClosed,
        ErrorCode::RevisionNotCurrent,
        ErrorCode::SelfApproval,
        ErrorCode::CommentState,
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
            ErrorCode::TransportOff => "transport_off",
            ErrorCode::UnknownRef => "unknown_ref",
            ErrorCode::UnknownKind => "unknown_kind",
            ErrorCode::BundleOfReceivedRef => "bundle_of_received_ref",
            ErrorCode::NotVerified => "not_verified",
            ErrorCode::ExportFailed => "export_failed",
            ErrorCode::RateLimited => "rate_limited",
            ErrorCode::RangeNotSatisfiable => "range_not_satisfiable",
            ErrorCode::Forbidden => "forbidden",
            ErrorCode::LengthRequired => "length_required",
            ErrorCode::PayloadTooLarge => "payload_too_large",
            ErrorCode::NeedsRecovery => "needs_recovery",
            ErrorCode::QuotaExceeded => "quota_exceeded",
            ErrorCode::BaseUnknown => "base_unknown",
            ErrorCode::DeclarationMissesAdded => "declaration_misses_added",
            ErrorCode::DeclaredNotHeld => "declared_not_held",
            ErrorCode::NoNewContent => "no_new_content",
            ErrorCode::UnknownProposal => "unknown_proposal",
            ErrorCode::ProposalNotOpen => "proposal_not_open",
            ErrorCode::NotYourProposal => "not_your_proposal",
            ErrorCode::AcceptRefused => "accept_refused",
            ErrorCode::IntentPending => "intent_pending",
            ErrorCode::Incomplete => "incomplete",
            ErrorCode::UnknownIntent => "unknown_intent",
            ErrorCode::ReasonRequired => "reason_required",
            ErrorCode::ProposalFinal => "proposal_final",
            ErrorCode::ArtifactRemoved => "artifact_removed",
            ErrorCode::CommentsClosed => "comments_closed",
            ErrorCode::DecisionsClosed => "decisions_closed",
            ErrorCode::RevisionNotCurrent => "revision_not_current",
            ErrorCode::SelfApproval => "self_approval",
            ErrorCode::CommentState => "comment_state",
        }
    }

    /// The HTTP status.
    pub fn status(self) -> StatusCode {
        match self {
            ErrorCode::NotFound | ErrorCode::RawElsewhere => StatusCode::NOT_FOUND,
            ErrorCode::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
            ErrorCode::TooLarge
            | ErrorCode::UnknownRef
            | ErrorCode::UnknownKind
            | ErrorCode::BundleOfReceivedRef => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::Unavailable
            | ErrorCode::Busy
            | ErrorCode::TransportOff
            | ErrorCode::NotVerified
            | ErrorCode::ExportFailed => StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::RangeNotSatisfiable => StatusCode::RANGE_NOT_SATISFIABLE,
            ErrorCode::Forbidden => StatusCode::FORBIDDEN,
            ErrorCode::LengthRequired => StatusCode::LENGTH_REQUIRED,
            ErrorCode::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            ErrorCode::NeedsRecovery => StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::QuotaExceeded => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::BaseUnknown => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::DeclarationMissesAdded => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::DeclaredNotHeld => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::NoNewContent => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::UnknownProposal => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::ProposalNotOpen => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::NotYourProposal => StatusCode::FORBIDDEN,
            ErrorCode::AcceptRefused => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::IntentPending => StatusCode::CONFLICT,
            ErrorCode::Incomplete => StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::UnknownIntent => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::ReasonRequired | ErrorCode::ProposalFinal => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            ErrorCode::ArtifactRemoved => StatusCode::GONE,
            ErrorCode::CommentsClosed
            | ErrorCode::DecisionsClosed
            | ErrorCode::RevisionNotCurrent
            | ErrorCode::SelfApproval
            | ErrorCode::CommentState => StatusCode::UNPROCESSABLE_ENTITY,
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
            ErrorCode::TransportOff => (Key::ErrTransportOffMessage, Key::ErrTransportOffHint),
            ErrorCode::UnknownRef => (Key::ErrUnknownRefMessage, Key::ErrUnknownRefHint),
            ErrorCode::UnknownKind => (Key::ErrUnknownKindMessage, Key::ErrUnknownKindHint),
            ErrorCode::BundleOfReceivedRef => (
                Key::ErrBundleOfReceivedMessage,
                Key::ErrBundleOfReceivedHint,
            ),
            ErrorCode::NotVerified => (Key::ErrNotVerifiedMessage, Key::ErrNotVerifiedHint),
            ErrorCode::ExportFailed => (Key::ErrExportFailedMessage, Key::ErrExportFailedHint),
            ErrorCode::RateLimited => (Key::ErrRateLimitedMessage, Key::ErrRateLimitedHint),
            ErrorCode::RangeNotSatisfiable => (Key::ErrRangeMessage, Key::ErrRangeHint),
            ErrorCode::Forbidden => (Key::ErrForbiddenMessage, Key::ErrForbiddenHint),
            ErrorCode::LengthRequired => {
                (Key::ErrLengthRequiredMessage, Key::ErrLengthRequiredHint)
            }
            ErrorCode::PayloadTooLarge => {
                (Key::ErrPayloadTooLargeMessage, Key::ErrPayloadTooLargeHint)
            }
            ErrorCode::NeedsRecovery => (Key::ErrNeedsRecoveryMessage, Key::ErrNeedsRecoveryHint),
            ErrorCode::QuotaExceeded => (Key::ErrQuotaExceededMessage, Key::ErrQuotaExceededHint),
            ErrorCode::BaseUnknown => (Key::ErrBaseUnknownMessage, Key::ErrBaseUnknownHint),
            ErrorCode::DeclarationMissesAdded => (
                Key::ErrDeclarationMissesAddedMessage,
                Key::ErrDeclarationMissesAddedHint,
            ),
            ErrorCode::DeclaredNotHeld => {
                (Key::ErrDeclaredNotHeldMessage, Key::ErrDeclaredNotHeldHint)
            }
            ErrorCode::NoNewContent => (Key::ErrNoNewContentMessage, Key::ErrNoNewContentHint),
            ErrorCode::UnknownProposal => {
                (Key::ErrUnknownProposalMessage, Key::ErrUnknownProposalHint)
            }
            ErrorCode::ProposalNotOpen => {
                (Key::ErrProposalNotOpenMessage, Key::ErrProposalNotOpenHint)
            }
            ErrorCode::NotYourProposal => {
                (Key::ErrNotYourProposalMessage, Key::ErrNotYourProposalHint)
            }
            ErrorCode::AcceptRefused => (Key::ErrAcceptRefusedMessage, Key::ErrAcceptRefusedHint),
            ErrorCode::IntentPending => (Key::ErrIntentPendingMessage, Key::ErrIntentPendingHint),
            ErrorCode::Incomplete => (Key::ErrIncompleteMessage, Key::ErrIncompleteHint),
            ErrorCode::UnknownIntent => (Key::ErrUnknownIntentMessage, Key::ErrUnknownIntentHint),
            ErrorCode::ReasonRequired => {
                (Key::ErrReasonRequiredMessage, Key::ErrReasonRequiredHint)
            }
            ErrorCode::ProposalFinal => (Key::ErrProposalFinalMessage, Key::ErrProposalFinalHint),
            ErrorCode::ArtifactRemoved => {
                (Key::ErrArtifactRemovedMessage, Key::ErrArtifactRemovedHint)
            }
            ErrorCode::CommentsClosed => {
                (Key::ErrCommentsClosedMessage, Key::ErrCommentsClosedHint)
            }
            ErrorCode::DecisionsClosed => {
                (Key::ErrDecisionsClosedMessage, Key::ErrDecisionsClosedHint)
            }
            ErrorCode::RevisionNotCurrent => (
                Key::ErrRevisionNotCurrentMessage,
                Key::ErrRevisionNotCurrentHint,
            ),
            ErrorCode::SelfApproval => (Key::ErrSelfApprovalMessage, Key::ErrSelfApprovalHint),
            ErrorCode::CommentState => (Key::ErrCommentStateMessage, Key::ErrCommentStateHint),
        };
        ErrorKeys { message, hint }
    }

    /// `too_large` from the raw route: the same code, status and message, and the hint that is true there
    /// (the content origin serves no more than the cap, so "download it from there" would be untrue).
    pub const TOO_LARGE_RAW: ErrorKeys = ErrorKeys {
        message: Key::ErrTooLargeMessage,
        hint: Key::ErrTooLargeRawHint,
    };

    /// `invalid_request` for a bundle fetch without a `ref`: the same code and status, and the hint that is true there.
    pub const REF_REQUIRED: ErrorKeys = ErrorKeys {
        message: Key::ErrInvalidRequestMessage,
        hint: Key::ErrRefRequiredHint,
    };

    /// `forbidden` on the administrator's door: the caller can read the repository but may not administer it.
    pub const FORBIDDEN_ADMIN: ErrorKeys = ErrorKeys {
        message: Key::ErrForbiddenAdminMessage,
        hint: Key::ErrForbiddenAdminHint,
    };

    /// `forbidden` on the download of an accepted artifact: the caller can read the repository but holds neither `review` nor
    /// `merge-seal`.
    pub const FORBIDDEN_REVIEW: ErrorKeys = ErrorKeys {
        message: Key::ErrForbiddenReviewMessage,
        hint: Key::ErrForbiddenReviewHint,
    };

    /// `forbidden` on closing a proposal: the caller can read the repository but is neither the opener nor allowed to close.
    pub const FORBIDDEN_CLOSE: ErrorKeys = ErrorKeys {
        message: Key::ErrForbiddenCloseMessage,
        hint: Key::ErrForbiddenCloseHint,
    };

    /// `forbidden` on commenting: the caller can read the repository but is not signed in (a comment has an author).
    pub const FORBIDDEN_COMMENT: ErrorKeys = ErrorKeys {
        message: Key::ErrForbiddenCommentMessage,
        hint: Key::ErrForbiddenCommentHint,
    };

    /// `forbidden` on a decision: the caller can read the repository but holds no `review` here.
    pub const FORBIDDEN_DECISION: ErrorKeys = ErrorKeys {
        message: Key::ErrForbiddenDecisionMessage,
        hint: Key::ErrForbiddenDecisionHint,
    };

    /// `forbidden` on hiding or showing a comment: the caller can read the repository but may not administer it.
    pub const FORBIDDEN_HIDE: ErrorKeys = ErrorKeys {
        message: Key::ErrForbiddenHideMessage,
        hint: Key::ErrForbiddenHideHint,
    };

    /// `invalid_request` for a comment that breaks a rule (the `reason` field says which).
    pub const BAD_COMMENT: ErrorKeys = ErrorKeys {
        message: Key::ErrInvalidRequestMessage,
        hint: Key::ErrCommentHint,
    };

    /// `invalid_request` for a decision that breaks a rule.
    pub const BAD_DECISION: ErrorKeys = ErrorKeys {
        message: Key::ErrInvalidRequestMessage,
        hint: Key::ErrDecisionHint,
    };

    /// `invalid_request` for hiding a comment without a proper reason.
    pub const BAD_HIDE: ErrorKeys = ErrorKeys {
        message: Key::ErrInvalidRequestMessage,
        hint: Key::ErrHideHint,
    };

    /// `rate_limited`: too many comments on one proposal in the last hour.
    pub const RATE_COMMENTS: ErrorKeys = ErrorKeys {
        message: Key::ErrRateCommentsMessage,
        hint: Key::ErrRateCommentsHint,
    };

    /// `rate_limited`: too many decisions on one proposal in the last hour.
    pub const RATE_DECISIONS: ErrorKeys = ErrorKeys {
        message: Key::ErrRateDecisionsMessage,
        hint: Key::ErrRateDecisionsHint,
    };

    /// `transport_off` for a proposal: fetches may be set up while the artifact store, which proposals need, is not.
    pub const PROPOSE_OFF: ErrorKeys = ErrorKeys {
        message: Key::ErrProposeOffMessage,
        hint: Key::ErrProposeOffHint,
    };

    /// `invalid_request` for a proposal whose body is not framed as the API document describes.
    pub const BAD_FRAMING: ErrorKeys = ErrorKeys {
        message: Key::ErrInvalidRequestMessage,
        hint: Key::ErrFramingHint,
    };

    /// `invalid_request` for a proposal whose metadata is not valid (the `reason` field says which part).
    pub const BAD_METADATA: ErrorKeys = ErrorKeys {
        message: Key::ErrInvalidRequestMessage,
        hint: Key::ErrMetadataHint,
    };

    /// `export_failed` because an export or verify call hit its wall-time bound.
    pub const EXPORT_DEADLINE: ErrorKeys = ErrorKeys {
        message: Key::ErrExportFailedMessage,
        hint: Key::ErrExportDeadlineHint,
    };

    /// `export_failed` because an export or verify call hit its memory ceiling.
    pub const EXPORT_CEILING: ErrorKeys = ErrorKeys {
        message: Key::ErrExportFailedMessage,
        hint: Key::ErrExportCeilingHint,
    };

    /// `export_failed` because the artifact is larger than the whole spool.
    pub const SPOOL_TOO_SMALL: ErrorKeys = ErrorKeys {
        message: Key::ErrExportFailedMessage,
        hint: Key::ErrSpoolTooSmallHint,
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
            ErrorCode::TransportOff => {
                "artifact fetches are not set up on this server (no export spool configured)"
            }
            ErrorCode::UnknownRef => "the repository has no ref of that name",
            ErrorCode::UnknownKind => "the fetch kind is neither `archive` nor `bundle`",
            ErrorCode::BundleOfReceivedRef => {
                "a bundle of a received ref was asked for; the archive carries received refs"
            }
            ErrorCode::NotVerified => "the exported artifact did not verify and was not served",
            ErrorCode::ExportFailed => {
                "the export failed, hit its deadline or its memory ceiling, or the artifact is larger than the spool (the hint names the cause)"
            }
            ErrorCode::RateLimited => {
                "too many requests from this principal or address (carries Retry-After)"
            }
            ErrorCode::RangeNotSatisfiable => {
                "a byte range the current artifact cannot satisfy (carries `Content-Range: bytes */size`)"
            }
            ErrorCode::Forbidden => "the caller can read the repository but may not propose to it",
            ErrorCode::LengthRequired => "a proposal must declare its length with `Content-Length`",
            ErrorCode::PayloadTooLarge => {
                "the proposal is larger than this server accepts (refused before the artifact is read)"
            }
            ErrorCode::NeedsRecovery => {
                "the repository is waiting for recovery after an unfinished write (reads are not affected)"
            }
            ErrorCode::QuotaExceeded => {
                "a limit on proposals was reached (the body also names the quota, its limit and the current use)"
            }
            ErrorCode::BaseUnknown => "the declared base is not in the repository's history",
            ErrorCode::DeclarationMissesAdded => {
                "the declared patches leave out patches the artifact added (the body lists them)"
            }
            ErrorCode::DeclaredNotHeld => {
                "the declared patches name patches the repository does not hold (the body lists them)"
            }
            ErrorCode::NoNewContent => {
                "the artifact added nothing the repository did not already hold"
            }
            ErrorCode::UnknownProposal => "the repository has no proposal with that id",
            ErrorCode::ProposalNotOpen => {
                "the proposal is landed or closed and takes no more revisions"
            }
            ErrorCode::NotYourProposal => {
                "the proposal was opened by somebody else and the caller does not hold `write`"
            }
            ErrorCode::AcceptRefused => {
                "prikk refused the artifact and left nothing behind (its own text is in the `detail` field)"
            }
            ErrorCode::IntentPending => {
                "somebody else has begun sending this exact artifact and it has not finished"
            }
            ErrorCode::Incomplete => {
                "the accept did not finish (the body names the intent; send the same artifact again once the repository has recovered)"
            }
            ErrorCode::UnknownIntent => {
                "an intent to abandon is not an open intent of this repository"
            }
            ErrorCode::ReasonRequired => "a proposal is closed with a reason, and none was given",
            ErrorCode::ProposalFinal => {
                "the proposal has landed or been closed (the body's `state` says which) and is closed no more"
            }
            ErrorCode::ArtifactRemoved => {
                "the files of that revision were kept until a date and have been removed since (the body's `kept_until` is that date)"
            }
            ErrorCode::CommentsClosed => {
                "the proposal finished more than its grace ago and takes no more comments (the body's `state` and `kept_until` say when)"
            }
            ErrorCode::DecisionsClosed => {
                "the proposal has landed or been closed (the body's `state` says which) and takes no more decisions"
            }
            ErrorCode::RevisionNotCurrent => {
                "a decision was named for a revision that is not the latest (the body's `current` is the latest)"
            }
            ErrorCode::SelfApproval => {
                "the submitter of a revision cannot approve it, and nobody who submitted any revision of a proposal can approve its latest (the body's `reason` says which)"
            }
            ErrorCode::CommentState => {
                "the comment is already hidden, or is not hidden (the body's `hidden` says which it is now)"
            }
        }
    }

    /// The JSON body in English: `{ "code", "message", "hint" }`.
    pub fn body(self) -> String {
        self.body_for(self.keys(), EN)
    }

    /// The JSON body for these keys in `lang`. One code, one status; the message and hint are the
    /// language-dependent fields.
    pub fn body_for(self, keys: ErrorKeys, lang: Lang) -> String {
        self.body_with(keys, lang, &ErrorDetail::default())
    }

    /// [`ErrorCode::body_for`] with a `detail`: its `params` fill the message and hint, and its `fields` are added to
    /// the object next to `code`, `message` and `hint` (never in place of them).
    pub fn body_with(self, keys: ErrorKeys, lang: Lang, detail: &ErrorDetail) -> String {
        let say = |key: Key| {
            let mut m = crate::catalog::T::new(lang)
                .msg(key)
                .with("document", crate::api::paths::OPENAPI);
            for (name, value) in &detail.params {
                m = m.with(name, value);
            }
            m.text()
        };
        let mut object = serde_json::Map::new();
        object.insert("code".to_owned(), self.code().into());
        object.insert("message".to_owned(), say(keys.message).into());
        object.insert("hint".to_owned(), say(keys.hint).into());
        for (name, value) in &detail.fields {
            object
                .entry((*name).to_owned())
                .or_insert_with(|| value.clone());
        }
        serde_json::Value::Object(object).to_string()
    }
}
