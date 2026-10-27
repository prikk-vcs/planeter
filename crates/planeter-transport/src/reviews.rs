//! Review over the records (RFC 005 part 6): comments, decisions, hiding, and what a proposal's page and API show of them.
//! (A *proposal* is the code's *change*; see `docs/proposals.md`.)
//!
//! **No prikk call is made here**, and no counter is kept: the rate limits are computed from the records (`rate_check` over
//! the timestamps the store returns), the review state from the current decisions on the latest revision, and the patch set
//! comments may anchor to from the revisions. The rules are `planeter_core::review`'s; this layer reads the facts they need,
//! applies them in a fixed order — **who may is the caller's job and is decided before any of this** — and writes.
//!
//! The order of the checks of a comment: the proposal exists in this repository; the window (open, or within the grace after
//! it finished); the comment's own rules (body, anchor, thread depth); only then the rate limit — a request that was refused
//! for a rule is never counted, because nothing was recorded.
//!
//! One race is accepted and bounded: a comment or decision that arrives while the proposal is being closed or landed may
//! be recorded just after the state changed. It is a comment (harmless) or a decision on a revision that is still the
//! latest; the state of the proposal is never touched from here.

use std::collections::BTreeMap;
use std::sync::Arc;

use planeter_core::change::current_patch_set;
use planeter_core::review::{
    COMMENTS_PER_HOUR_DEFAULT, CommentInput, CommentRefusal, CommentsClosed, CurrentDecision,
    DECISIONS_PER_HOUR_DEFAULT, DecisionContext, DecisionInput, DecisionRefusal, HideRefusal,
    ParentRef, RATE_WINDOW_SECS, ReviewState, Standing, Verdict, check_comment,
    check_comment_window, check_decision, check_hide_reason, rate_check, review_standing,
};
use planeter_store::{
    ChangeRecord, ChangeStore, CommentRecord, DecisionRecord, RepositoryRecord, ReviewStore,
    RevisionRecord, StoreError,
};

/// How many comment threads one page of the detail holds at most.
pub const THREADS_LIMIT_MAX: usize = 100;
/// The default page.
pub const THREADS_LIMIT_DEFAULT: usize = 50;

/// Operator settings of review.
#[derive(Debug, Clone, Copy)]
pub struct ReviewConfig {
    /// Comments per principal per proposal per hour.
    pub comments_per_hour: u64,
    /// Decisions per principal per proposal per hour.
    pub decisions_per_hour: u64,
}

impl Default for ReviewConfig {
    fn default() -> Self {
        ReviewConfig {
            comments_per_hour: COMMENTS_PER_HOUR_DEFAULT,
            decisions_per_hour: DECISIONS_PER_HOUR_DEFAULT,
        }
    }
}

/// What a rate limit was about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limited {
    Comments,
    Decisions,
}

/// Why a review request did not succeed.
#[derive(Debug)]
pub enum ReviewError {
    /// No such proposal **in this repository** (an id of another repository is the same as an unknown one), or no such comment.
    NotFound,
    /// The `after` cursor is not one this service gave.
    BadCursor,
    Comment(CommentRefusal),
    CommentsClosed(CommentsClosed),
    Decision(DecisionRefusal),
    HideReason(HideRefusal),
    /// The comment is already in the state asked for (`hidden` says which: already hidden / not hidden).
    CommentState {
        hidden: bool,
    },
    /// Too many in the last hour; the seconds until there is room.
    RateLimited {
        what: Limited,
        retry_after: u64,
    },
    /// The records could not be read or written.
    Unavailable(String),
}

fn unavailable(what: &str, e: impl std::fmt::Display) -> ReviewError {
    eprintln!("planeter: review: {what}: {e}");
    ReviewError::Unavailable(what.to_owned())
}

/// A comment as sent (the author is the caller's principal, not part of the request).
#[derive(Debug, Clone, Default)]
pub struct NewComment {
    pub body: String,
    pub patch_id: Option<String>,
    pub path: Option<String>,
    pub line_from: Option<u32>,
    pub line_to: Option<u32>,
    pub parent: Option<String>,
}

/// One current decision, as the page and the API show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionView {
    pub reviewer: String,
    pub revision: u32,
    pub verdict: Verdict,
    pub note: Option<String>,
    pub created_at: u64,
}

/// The review of a proposal: the state and everything the page shows of the decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewView {
    pub latest: u32,
    pub standing: Standing,
    /// The current decisions **on the latest revision**, by reviewer.
    pub current: Vec<DecisionView>,
    /// The current decisions on earlier revisions, by revision then reviewer: history, counted for nothing.
    pub earlier: Vec<DecisionView>,
}

/// A top-level comment with its replies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    pub comment: CommentRecord,
    pub replies: Vec<CommentRecord>,
}

/// A page of threads, oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadsPage {
    pub threads: Vec<Thread>,
    /// The cursor to pass as `after` for the next page; absent on the last page.
    pub next: Option<String>,
    /// All the proposal's comments, replies included.
    pub total: u64,
}

fn new_id() -> String {
    let token = planeter_auth::random_token();
    format!("m-{}", &planeter_auth::sha256_hex(token.as_bytes())[..24])
}

fn system_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn cursor(c: &CommentRecord) -> String {
    format!("{}.{}", c.created_at, c.comment_id)
}

fn parse_cursor(s: &str) -> Result<(u64, String), ReviewError> {
    let (t, id) = s.split_once('.').ok_or(ReviewError::BadCursor)?;
    let t: u64 = t.parse().map_err(|_| ReviewError::BadCursor)?;
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(ReviewError::BadCursor);
    }
    Ok((t, id.to_owned()))
}

/// The review service. Cheap to share (`Arc`); every method is **blocking** (call it from `spawn_blocking`).
pub struct ReviewService {
    changes: Arc<dyn ChangeStore>,
    reviews: Arc<dyn ReviewStore>,
    config: ReviewConfig,
    now_unix: fn() -> u64,
}

impl ReviewService {
    pub fn new(
        changes: Arc<dyn ChangeStore>,
        reviews: Arc<dyn ReviewStore>,
        config: ReviewConfig,
    ) -> Self {
        ReviewService {
            changes,
            reviews,
            config,
            now_unix: system_now,
        }
    }

    /// Replace the clock (tests pin it).
    #[must_use]
    pub fn with_clock(mut self, now_unix: fn() -> u64) -> Self {
        self.now_unix = now_unix;
        self
    }

    pub fn config(&self) -> &ReviewConfig {
        &self.config
    }

    /// The proposal `id` if it belongs to this repository, with its revisions in order.
    fn own(
        &self,
        record: &RepositoryRecord,
        id: &str,
    ) -> Result<(ChangeRecord, Vec<RevisionRecord>), ReviewError> {
        let change = match self.changes.get_change(id) {
            Ok(Some(c)) if c.repo_id == record.repo_id => c,
            Ok(_) => return Err(ReviewError::NotFound),
            Err(e) => return Err(unavailable("cannot read the proposal", e)),
        };
        let revisions = self
            .changes
            .revisions_of(&change.change_id)
            .map_err(|e| unavailable("cannot read the revisions", e))?;
        Ok((change, revisions))
    }

    fn latest(revisions: &[RevisionRecord]) -> u32 {
        revisions.iter().map(|r| r.seq).max().unwrap_or(1)
    }

    // ------------------------------------------------------------------------------------------ comments

    /// Add a comment. The caller has already decided that `author` may comment (`authorize(Comment)`).
    pub fn add_comment(
        &self,
        record: &RepositoryRecord,
        id: &str,
        author: &str,
        new: &NewComment,
    ) -> Result<CommentRecord, ReviewError> {
        let (change, revisions) = self.own(record, id)?;
        let now = (self.now_unix)();
        check_comment_window(change.state, change.retain_until, now)
            .map_err(ReviewError::CommentsClosed)?;
        let patch_set = current_patch_set(&revisions);
        let parent = match new.parent.as_deref() {
            None => ParentRef::None,
            Some(pid) => match self.reviews.get_comment(pid) {
                Ok(None) => ParentRef::Missing,
                Ok(Some(p)) => ParentRef::Found {
                    in_this_proposal: p.change_id == change.change_id,
                    top_level: p.parent.is_none(),
                },
                Err(e) => return Err(unavailable("cannot read the parent comment", e)),
            },
        };
        let anchor = check_comment(
            &CommentInput {
                body: &new.body,
                patch_id: new.patch_id.as_deref(),
                path: new.path.as_deref(),
                line_from: new.line_from,
                line_to: new.line_to,
                parent,
            },
            &patch_set,
        )
        .map_err(ReviewError::Comment)?;
        // The limit is computed from the records: what this principal wrote on this proposal in the last hour.
        let times = self
            .reviews
            .comment_times(
                &change.change_id,
                author,
                now.saturating_sub(RATE_WINDOW_SECS),
            )
            .map_err(|e| unavailable("cannot count the comments", e))?;
        rate_check(&times, self.config.comments_per_hour, now).map_err(|retry_after| {
            ReviewError::RateLimited {
                what: Limited::Comments,
                retry_after,
            }
        })?;
        let comment = CommentRecord {
            comment_id: new_id(),
            change_id: change.change_id.clone(),
            revision_seq: Self::latest(&revisions),
            patch_id: anchor.patch_id,
            path: anchor.path,
            line_from: anchor.lines.map(|l| l.0),
            line_to: anchor.lines.map(|l| l.1),
            parent: new.parent.clone(),
            author: author.to_owned(),
            body: new.body.clone(),
            created_at: now,
            hidden: None,
        };
        self.reviews.add_comment(&comment).map_err(|e| match e {
            // The parent changed under us (cannot happen: comments are never deleted) or a rule of the store.
            StoreError::Invalid(_) => ReviewError::Comment(CommentRefusal::ParentNotTopLevel),
            e => unavailable("cannot record the comment", e),
        })?;
        eprintln!(
            "planeter: review: comment repo={} proposal={} comment={}",
            record.repo_id.as_str(),
            change.change_id,
            comment.comment_id
        );
        Ok(comment)
    }

    /// Hide a comment of this repository's proposal, with a reason. The caller has already decided that `by` holds `RepoAdmin`.
    pub fn hide(
        &self,
        record: &RepositoryRecord,
        id: &str,
        comment_id: &str,
        by: &str,
        reason: &str,
    ) -> Result<CommentRecord, ReviewError> {
        let (change, _) = self.own(record, id)?;
        let reason = check_hide_reason(reason).map_err(ReviewError::HideReason)?;
        self.comment_of(&change, comment_id)?;
        match self
            .reviews
            .hide_comment(comment_id, by, &reason, (self.now_unix)())
        {
            Ok(()) => {}
            Err(StoreError::Duplicate(_)) => {
                return Err(ReviewError::CommentState { hidden: true });
            }
            Err(StoreError::NotFound) => return Err(ReviewError::NotFound),
            Err(e) => return Err(unavailable("cannot hide the comment", e)),
        }
        eprintln!(
            "planeter: review: hidden repo={} proposal={} comment={} by={by}",
            record.repo_id.as_str(),
            change.change_id,
            comment_id
        );
        self.comment_of(&change, comment_id)
    }

    /// Make a hidden comment visible again.
    pub fn unhide(
        &self,
        record: &RepositoryRecord,
        id: &str,
        comment_id: &str,
        by: &str,
    ) -> Result<CommentRecord, ReviewError> {
        let (change, _) = self.own(record, id)?;
        self.comment_of(&change, comment_id)?;
        match self
            .reviews
            .unhide_comment(comment_id, by, (self.now_unix)())
        {
            Ok(()) => {}
            Err(StoreError::Duplicate(_)) => {
                return Err(ReviewError::CommentState { hidden: false });
            }
            Err(StoreError::NotFound) => return Err(ReviewError::NotFound),
            Err(e) => return Err(unavailable("cannot unhide the comment", e)),
        }
        eprintln!(
            "planeter: review: unhidden repo={} proposal={} comment={} by={by}",
            record.repo_id.as_str(),
            change.change_id,
            comment_id
        );
        self.comment_of(&change, comment_id)
    }

    /// A comment of this proposal: another proposal's comment is the same as an unknown one.
    fn comment_of(
        &self,
        change: &ChangeRecord,
        comment_id: &str,
    ) -> Result<CommentRecord, ReviewError> {
        match self.reviews.get_comment(comment_id) {
            Ok(Some(c)) if c.change_id == change.change_id => Ok(c),
            Ok(_) => Err(ReviewError::NotFound),
            Err(e) => Err(unavailable("cannot read the comment", e)),
        }
    }

    /// The proposal's comments, `limit` threads at a time (oldest first), each with all its replies.
    pub fn threads(
        &self,
        record: &RepositoryRecord,
        id: &str,
        limit: usize,
        after: Option<&str>,
    ) -> Result<ThreadsPage, ReviewError> {
        let (change, _) = self.own(record, id)?;
        self.threads_of(&change, limit, after)
    }

    fn threads_of(
        &self,
        change: &ChangeRecord,
        limit: usize,
        after: Option<&str>,
    ) -> Result<ThreadsPage, ReviewError> {
        let after = after.map(parse_cursor).transpose()?;
        let limit = limit.clamp(1, THREADS_LIMIT_MAX);
        // One more than asked, to know whether there is a next page.
        let mut tops = self
            .reviews
            .comment_threads(
                &change.change_id,
                after.as_ref().map(|(t, id)| (*t, id.as_str())),
                limit + 1,
            )
            .map_err(|e| unavailable("cannot read the comments", e))?;
        let next = if tops.len() > limit {
            tops.truncate(limit);
            tops.last().map(cursor)
        } else {
            None
        };
        let ids: Vec<String> = tops.iter().map(|c| c.comment_id.clone()).collect();
        let replies = self
            .reviews
            .replies_of(&change.change_id, &ids)
            .map_err(|e| unavailable("cannot read the replies", e))?;
        let mut by_parent: BTreeMap<String, Vec<CommentRecord>> = BTreeMap::new();
        for r in replies {
            if let Some(p) = r.parent.clone() {
                by_parent.entry(p).or_default().push(r);
            }
        }
        let threads = tops
            .into_iter()
            .map(|c| Thread {
                replies: by_parent.remove(&c.comment_id).unwrap_or_default(),
                comment: c,
            })
            .collect();
        let total = self
            .reviews
            .count_comments(&change.change_id)
            .map_err(|e| unavailable("cannot count the comments", e))?;
        Ok(ThreadsPage {
            threads,
            next,
            total,
        })
    }

    // ------------------------------------------------------------------------------------------ decisions

    /// Record a decision. The caller has already decided that `reviewer` may review (`authorize(Review)`).
    pub fn decide(
        &self,
        record: &RepositoryRecord,
        id: &str,
        reviewer: &str,
        verdict: Verdict,
        revision: Option<u32>,
        note: Option<&str>,
    ) -> Result<DecisionRecord, ReviewError> {
        let (change, revisions) = self.own(record, id)?;
        let submitters: Vec<(u32, String)> = revisions
            .iter()
            .map(|r| (r.seq, r.submitter.clone()))
            .collect();
        let note = note.map(str::trim).filter(|n| !n.is_empty());
        let seq = check_decision(
            &DecisionInput {
                reviewer,
                verdict,
                revision,
                note,
            },
            &DecisionContext {
                state: change.state,
                latest: Self::latest(&revisions),
                submitters: &submitters,
            },
        )
        .map_err(ReviewError::Decision)?;
        let now = (self.now_unix)();
        let times = self
            .reviews
            .decision_times(
                &change.change_id,
                reviewer,
                now.saturating_sub(RATE_WINDOW_SECS),
            )
            .map_err(|e| unavailable("cannot count the decisions", e))?;
        rate_check(&times, self.config.decisions_per_hour, now).map_err(|retry_after| {
            ReviewError::RateLimited {
                what: Limited::Decisions,
                retry_after,
            }
        })?;
        let d = DecisionRecord {
            change_id: change.change_id.clone(),
            revision_seq: seq,
            reviewer: reviewer.to_owned(),
            verdict: verdict.as_str().to_owned(),
            note: note.map(str::to_owned),
            created_at: now,
        };
        self.reviews
            .record_decision(&d)
            .map_err(|e| unavailable("cannot record the decision", e))?;
        eprintln!(
            "planeter: review: decision repo={} proposal={} revision={seq} reviewer={reviewer} verdict={}",
            record.repo_id.as_str(),
            change.change_id,
            verdict.as_str()
        );
        Ok(d)
    }

    fn views(records: Vec<DecisionRecord>) -> Vec<DecisionView> {
        records
            .into_iter()
            .filter_map(|d| {
                Some(DecisionView {
                    verdict: Verdict::parse(&d.verdict)?,
                    reviewer: d.reviewer,
                    revision: d.revision_seq,
                    note: d.note,
                    created_at: d.created_at,
                })
            })
            .collect()
    }

    /// The review of a proposal of this repository.
    pub fn review(&self, record: &RepositoryRecord, id: &str) -> Result<ReviewView, ReviewError> {
        let (change, revisions) = self.own(record, id)?;
        self.review_of(&change, &revisions)
    }

    /// The review of a proposal already read (the detail has it).
    pub fn review_of(
        &self,
        change: &ChangeRecord,
        revisions: &[RevisionRecord],
    ) -> Result<ReviewView, ReviewError> {
        let latest = Self::latest(revisions);
        let all = Self::views(
            self.reviews
                .current_decisions(&change.change_id)
                .map_err(|e| unavailable("cannot read the decisions", e))?,
        );
        let current_decisions: Vec<CurrentDecision> = all
            .iter()
            .map(|d| CurrentDecision {
                reviewer: d.reviewer.clone(),
                revision: d.revision,
                verdict: d.verdict,
            })
            .collect();
        let standing = review_standing(&current_decisions, latest);
        let (current, earlier): (Vec<_>, Vec<_>) = all
            .into_iter()
            .filter(|d| d.revision <= latest)
            .partition(|d| d.revision == latest);
        Ok(ReviewView {
            latest,
            standing,
            current,
            earlier,
        })
    }

    /// The comments and the review of a proposal already read, for the detail.
    pub fn detail_parts(
        &self,
        change: &ChangeRecord,
        revisions: &[RevisionRecord],
        limit: usize,
        after: Option<&str>,
    ) -> Result<(ReviewView, ThreadsPage), ReviewError> {
        Ok((
            self.review_of(change, revisions)?,
            self.threads_of(change, limit, after)?,
        ))
    }

    /// The review word of each proposal, for the list: `(change id, latest revision)` in, the state out. A failure is "no
    /// word" (the list still answers), never an error.
    pub fn review_words(&self, items: &[(String, u32)]) -> BTreeMap<String, ReviewState> {
        let decisions = match self.reviews.decisions_on(items) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("planeter: review: no review words this time: {e}");
                return BTreeMap::new();
            }
        };
        let mut by_change: BTreeMap<&str, Vec<CurrentDecision>> = BTreeMap::new();
        for d in &decisions {
            if let Some(v) = Verdict::parse(&d.verdict) {
                by_change
                    .entry(d.change_id.as_str())
                    .or_default()
                    .push(CurrentDecision {
                        reviewer: d.reviewer.clone(),
                        revision: d.revision_seq,
                        verdict: v,
                    });
            }
        }
        items
            .iter()
            .map(|(id, seq)| {
                let ds = by_change.get(id.as_str()).map(Vec::as_slice).unwrap_or(&[]);
                (id.clone(), review_standing(ds, *seq).state)
            })
            .collect()
    }
}
