//! The pure rules of review (RFC 005 part 6; D-5, D-7, D-10, D-11): who may say what about a proposal, when, and what a pile of
//! decisions amounts to. **No I/O**: every function takes the facts it needs (the records read by the caller) and returns a
//! verdict; nothing here reads a clock, a store or prikk. (A *proposal* is the code's *change*; see `docs/proposals.md`.)
//!
//! # The rules, in plain words
//!
//! 1. **Who.** A comment needs `authorize(Comment)`, a decision `authorize(Review)`, hiding `authorize(RepoAdmin)`; all three
//!    need the repository to be readable by the caller. (Deciding who is allowed is the caller's; this module decides what
//!    is *accepted*.)
//! 2. **When.** Comments are accepted on an `open` proposal and, for discussion after the fact, on a `landed` or `closed`
//!    one until its `retain_until` (the retention grace, 30 days by default) — [`check_comment_window`]. Decisions are
//!    accepted only while the proposal is `open` — [`check_decision`].
//! 3. **No self-approval.** The submitter of a revision cannot `approve` it, and a person who submitted *any* revision of a
//!    proposal cannot approve its latest one either. `request-changes` and comments are allowed.
//! 4. **A decision belongs to one revision**, by default the latest; naming another is refused. What a later revision shows
//!    of an earlier approval is "approved on revision N", and it counts for nothing now ([`review_standing`], built on
//!    [`approval_standing`](crate::change::approval_standing)).
//! 5. **The review state** is a function of the current decisions on the **latest** revision and of nothing else
//!    ([`review_standing`]): `changes_requested` if any reviewer's current decision on it is `request-changes`, else
//!    `approved` if at least one is `approve`, else `awaiting_review`. Counts are of distinct reviewers.
//! 6. **Comments are immutable and never deleted**; the only moderation is hiding, with a reason, by `RepoAdmin`, reversible
//!    ([`check_hide_reason`]).
//! 7. **Rate limits are computed from the records** (no counter that can drift): at most N comments and M decisions per
//!    principal per proposal per hour ([`rate_check`]).

use std::collections::{BTreeMap, BTreeSet};

use planeter_store::ChangeState;

use crate::change::{ApprovalStanding, approval_standing};

/// The longest comment body, in bytes of Markdown source (16 KiB).
pub const COMMENT_BODY_MAX_BYTES: usize = 16 * 1024;
/// The longest path an anchor names, in characters. Free text, never opened.
pub const COMMENT_PATH_MAX_CHARS: usize = 1024;
/// The longest note on a decision, in bytes of plain text (2 KiB).
pub const DECISION_NOTE_MAX_BYTES: usize = 2 * 1024;
/// The longest reason for hiding a comment, in characters.
pub const HIDE_REASON_MAX_CHARS: usize = 500;
/// Comments per principal per proposal per hour (the default of the operator variable).
pub const COMMENTS_PER_HOUR_DEFAULT: u64 = 20;
/// Decisions per principal per proposal per hour (the default of the operator variable).
pub const DECISIONS_PER_HOUR_DEFAULT: u64 = 10;
/// The window of the rate limits, in seconds.
pub const RATE_WINDOW_SECS: u64 = 3600;

// ------------------------------------------------------------------------------------------------ decisions and the state

/// What a reviewer says about a revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    Approve,
    RequestChanges,
}

impl Verdict {
    /// `approve` or `request-changes` (the API's and the store's word).
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Approve => "approve",
            Verdict::RequestChanges => "request-changes",
        }
    }
    pub fn parse(s: &str) -> Option<Verdict> {
        match s {
            "approve" => Some(Verdict::Approve),
            "request-changes" => Some(Verdict::RequestChanges),
            _ => None,
        }
    }
}

/// The review state of a proposal: three words, a function of the current decisions on its latest revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewState {
    AwaitingReview,
    Approved,
    ChangesRequested,
}

impl ReviewState {
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewState::AwaitingReview => "awaiting_review",
            ReviewState::Approved => "approved",
            ReviewState::ChangesRequested => "changes_requested",
        }
    }
}

/// One reviewer's **current** decision on one revision (a replaced decision is not one: the store keeps it as history).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentDecision {
    pub reviewer: String,
    pub revision: u32,
    pub verdict: Verdict,
}

/// What the decisions amount to for a proposal whose latest revision is `latest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Standing {
    pub state: ReviewState,
    /// Distinct reviewers whose current decision on the **latest** revision is `approve`, sorted.
    pub approved_by: Vec<String>,
    /// Distinct reviewers whose current decision on the latest revision is `request-changes`, sorted.
    pub changes_requested_by: Vec<String>,
    /// The **earlier** revisions that hold at least one current `approve`: history, counted for nothing (D-7). Ascending.
    pub approved_on: Vec<u32>,
    /// The earlier revisions that hold at least one current `request-changes`. Ascending.
    pub requested_changes_on: Vec<u32>,
}

impl Standing {
    /// The only approvals are for earlier revisions: the page says so ("Approved on an earlier revision; …").
    pub fn approved_only_earlier(&self) -> bool {
        self.approved_by.is_empty() && !self.approved_on.is_empty()
    }
}

/// The review state and its history from the current decisions of every revision. Decisions on a revision after `latest`
/// cannot exist and are ignored. Counts are of distinct reviewers.
pub fn review_standing(decisions: &[CurrentDecision], latest: u32) -> Standing {
    let mut approved: BTreeSet<&str> = BTreeSet::new();
    let mut requested: BTreeSet<&str> = BTreeSet::new();
    let mut approved_on: BTreeSet<u32> = BTreeSet::new();
    let mut requested_on: BTreeSet<u32> = BTreeSet::new();
    for d in decisions.iter().filter(|d| d.revision <= latest) {
        match (d.verdict, approval_standing(d.revision, latest)) {
            (Verdict::Approve, ApprovalStanding::Current) => {
                approved.insert(&d.reviewer);
            }
            (Verdict::Approve, ApprovalStanding::OnEarlierRevision(n)) => {
                approved_on.insert(n);
            }
            (Verdict::RequestChanges, ApprovalStanding::Current) => {
                requested.insert(&d.reviewer);
            }
            (Verdict::RequestChanges, ApprovalStanding::OnEarlierRevision(n)) => {
                requested_on.insert(n);
            }
        }
    }
    let state = if !requested.is_empty() {
        ReviewState::ChangesRequested
    } else if !approved.is_empty() {
        ReviewState::Approved
    } else {
        ReviewState::AwaitingReview
    };
    Standing {
        state,
        approved_by: approved.into_iter().map(str::to_owned).collect(),
        changes_requested_by: requested.into_iter().map(str::to_owned).collect(),
        approved_on: approved_on.into_iter().collect(),
        requested_changes_on: requested_on.into_iter().collect(),
    }
}

// ------------------------------------------------------------------------------------------------ comments

/// What the caller read about a comment's parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentRef {
    /// The comment is a thread's first.
    None,
    /// A parent was named and there is no such comment.
    Missing,
    /// The parent exists.
    Found {
        /// It belongs to this proposal.
        in_this_proposal: bool,
        /// It is itself a top-level comment (a thread is one level deep).
        top_level: bool,
    },
}

/// A comment as sent, before it is accepted.
#[derive(Debug, Clone, Copy)]
pub struct CommentInput<'a> {
    pub body: &'a str,
    pub patch_id: Option<&'a str>,
    pub path: Option<&'a str>,
    pub line_from: Option<u32>,
    pub line_to: Option<u32>,
    pub parent: ParentRef,
}

/// Why a comment is refused. Each has a stable `reason` word the API puts in `invalid_request`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentRefusal {
    BodyEmpty,
    BodyTooLong,
    PatchIdInvalid,
    /// The patch is not in the proposal's current patch set.
    PatchNotInSet,
    PathEmpty,
    PathTooLong,
    /// The path holds a control character (it is text to show, never a file name to open).
    PathInvalid,
    /// Lines were named without a path.
    LinesNeedAPath,
    /// `line_to` without `line_from`.
    LinesIncomplete,
    LineZero,
    LineOrder,
    ParentUnknown,
    ParentOtherProposal,
    /// A reply to a reply: a thread is one level deep.
    ParentNotTopLevel,
}

impl CommentRefusal {
    pub fn reason(self) -> &'static str {
        match self {
            CommentRefusal::BodyEmpty => "body_empty",
            CommentRefusal::BodyTooLong => "body_too_long",
            CommentRefusal::PatchIdInvalid => "patch_id_invalid",
            CommentRefusal::PatchNotInSet => "patch_not_in_set",
            CommentRefusal::PathEmpty => "path_empty",
            CommentRefusal::PathTooLong => "path_too_long",
            CommentRefusal::PathInvalid => "path_invalid",
            CommentRefusal::LinesNeedAPath => "lines_need_a_path",
            CommentRefusal::LinesIncomplete => "lines_incomplete",
            CommentRefusal::LineZero => "line_zero",
            CommentRefusal::LineOrder => "line_order",
            CommentRefusal::ParentUnknown => "parent_unknown",
            CommentRefusal::ParentOtherProposal => "parent_other_proposal",
            CommentRefusal::ParentNotTopLevel => "parent_not_top_level",
        }
    }
}

fn is_patch_id(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The anchor of an accepted comment, normalised: a lone `line_from` is the single line `line_from..=line_from`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    pub patch_id: Option<String>,
    pub path: Option<String>,
    pub lines: Option<(u32, u32)>,
}

/// Check a comment against the rules and give back its normalised anchor. `patch_set` is the proposal's **current** patch
/// set at this moment (the patch id is kept as written afterwards, even when a later revision drops it).
pub fn check_comment(
    input: &CommentInput<'_>,
    patch_set: &BTreeSet<String>,
) -> Result<Anchor, CommentRefusal> {
    if input.body.trim().is_empty() {
        return Err(CommentRefusal::BodyEmpty);
    }
    if input.body.len() > COMMENT_BODY_MAX_BYTES {
        return Err(CommentRefusal::BodyTooLong);
    }
    if let Some(p) = input.patch_id {
        if !is_patch_id(p) {
            return Err(CommentRefusal::PatchIdInvalid);
        }
        if !patch_set.contains(p) {
            return Err(CommentRefusal::PatchNotInSet);
        }
    }
    if let Some(path) = input.path {
        if path.trim().is_empty() {
            return Err(CommentRefusal::PathEmpty);
        }
        if path.chars().count() > COMMENT_PATH_MAX_CHARS {
            return Err(CommentRefusal::PathTooLong);
        }
        if path.chars().any(char::is_control) {
            return Err(CommentRefusal::PathInvalid);
        }
    }
    let lines = match (input.line_from, input.line_to) {
        (None, None) => None,
        (None, Some(_)) => return Err(CommentRefusal::LinesIncomplete),
        (Some(from), to) => {
            if input.path.is_none() {
                return Err(CommentRefusal::LinesNeedAPath);
            }
            let to = to.unwrap_or(from);
            if from == 0 || to == 0 {
                return Err(CommentRefusal::LineZero);
            }
            if from > to {
                return Err(CommentRefusal::LineOrder);
            }
            Some((from, to))
        }
    };
    match input.parent {
        ParentRef::None => {}
        ParentRef::Missing => return Err(CommentRefusal::ParentUnknown),
        ParentRef::Found {
            in_this_proposal: false,
            ..
        } => return Err(CommentRefusal::ParentOtherProposal),
        ParentRef::Found {
            top_level: false, ..
        } => return Err(CommentRefusal::ParentNotTopLevel),
        ParentRef::Found { .. } => {}
    }
    Ok(Anchor {
        patch_id: input.patch_id.map(str::to_owned),
        path: input.path.map(str::to_owned),
        lines,
    })
}

/// A comment is refused because the proposal finished too long ago.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommentsClosed {
    pub state: ChangeState,
    /// Until when comments were accepted (unix seconds): the proposal's `retain_until`.
    pub until: u64,
}

/// Comments are accepted on an open proposal and, until `retain_until` (inclusive), on a landed or closed one. A finished
/// proposal with no `retain_until` accepts none.
pub fn check_comment_window(
    state: ChangeState,
    retain_until: Option<u64>,
    now: u64,
) -> Result<(), CommentsClosed> {
    match state {
        ChangeState::Open => Ok(()),
        ChangeState::Landed | ChangeState::Closed => match retain_until {
            Some(until) if now <= until => Ok(()),
            until => Err(CommentsClosed {
                state,
                until: until.unwrap_or(0),
            }),
        },
    }
}

// ------------------------------------------------------------------------------------------------ decisions

/// Which of the two self-approval rules applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfApproval {
    /// The reviewer submitted the revision they are approving.
    OwnRevision,
    /// The reviewer submitted another revision of the same proposal.
    OwnProposal,
}

impl SelfApproval {
    pub fn reason(self) -> &'static str {
        match self {
            SelfApproval::OwnRevision => "own_revision",
            SelfApproval::OwnProposal => "own_proposal",
        }
    }
}

/// Why a decision is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionRefusal {
    /// The proposal has landed or been closed: decisions are accepted only while it is open.
    NotOpen(ChangeState),
    /// A revision other than the latest was named.
    RevisionNotCurrent {
        current: u32,
    },
    SelfApproval(SelfApproval),
    NoteTooLong,
}

/// A decision as sent.
#[derive(Debug, Clone, Copy)]
pub struct DecisionInput<'a> {
    pub reviewer: &'a str,
    pub verdict: Verdict,
    /// The revision named; `None` is the latest.
    pub revision: Option<u32>,
    pub note: Option<&'a str>,
}

/// What the proposal looks like to a decision: its state, its latest revision, and who submitted each revision.
#[derive(Debug, Clone, Copy)]
pub struct DecisionContext<'a> {
    pub state: ChangeState,
    pub latest: u32,
    /// `(seq, submitter)` of every revision.
    pub submitters: &'a [(u32, String)],
}

/// Check a decision; the revision it belongs to when accepted. Order of the checks: state, revision, self-approval, note.
pub fn check_decision(
    input: &DecisionInput<'_>,
    ctx: &DecisionContext<'_>,
) -> Result<u32, DecisionRefusal> {
    if ctx.state != ChangeState::Open {
        return Err(DecisionRefusal::NotOpen(ctx.state));
    }
    let revision = input.revision.unwrap_or(ctx.latest);
    if revision != ctx.latest {
        return Err(DecisionRefusal::RevisionNotCurrent {
            current: ctx.latest,
        });
    }
    if input.verdict == Verdict::Approve {
        if ctx
            .submitters
            .iter()
            .any(|(seq, who)| *seq == revision && who == input.reviewer)
        {
            return Err(DecisionRefusal::SelfApproval(SelfApproval::OwnRevision));
        }
        if ctx.submitters.iter().any(|(_, who)| who == input.reviewer) {
            return Err(DecisionRefusal::SelfApproval(SelfApproval::OwnProposal));
        }
    }
    if input
        .note
        .is_some_and(|n| n.len() > DECISION_NOTE_MAX_BYTES)
    {
        return Err(DecisionRefusal::NoteTooLong);
    }
    Ok(revision)
}

// ------------------------------------------------------------------------------------------------ moderation, limits

/// Why a hide is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HideRefusal {
    ReasonRequired,
    ReasonTooLong,
}

/// The reason for hiding a comment, trimmed: required, at most [`HIDE_REASON_MAX_CHARS`] characters.
pub fn check_hide_reason(reason: &str) -> Result<String, HideRefusal> {
    let r = reason.trim();
    if r.is_empty() {
        return Err(HideRefusal::ReasonRequired);
    }
    if r.chars().count() > HIDE_REASON_MAX_CHARS {
        return Err(HideRefusal::ReasonTooLong);
    }
    Ok(r.to_owned())
}

/// A per-principal, per-proposal limit: `times` are when the principal did the thing (unix seconds, any order). At most
/// `limit` within the last [`RATE_WINDOW_SECS`]; refused with the seconds until the oldest one leaves the window.
pub fn rate_check(times: &[u64], limit: u64, now: u64) -> Result<(), u64> {
    let since = now.saturating_sub(RATE_WINDOW_SECS);
    let mut inside: Vec<u64> = times.iter().copied().filter(|t| *t > since).collect();
    if (inside.len() as u64) < limit {
        return Ok(());
    }
    inside.sort_unstable();
    // The limit-th most recent one decides: when it leaves the window there is room again.
    let deciding = inside[inside.len() - limit.max(1) as usize];
    Err((deciding + RATE_WINDOW_SECS).saturating_sub(now).max(1))
}

/// For the list: the review word of each proposal from its current decisions, `(change id → decisions)`.
pub fn review_states_of(
    decisions: &BTreeMap<String, Vec<CurrentDecision>>,
    latest: &BTreeMap<String, u32>,
) -> BTreeMap<String, ReviewState> {
    latest
        .iter()
        .map(|(id, seq)| {
            let ds = decisions.get(id).map(Vec::as_slice).unwrap_or(&[]);
            (id.clone(), review_standing(ds, *seq).state)
        })
        .collect()
}
