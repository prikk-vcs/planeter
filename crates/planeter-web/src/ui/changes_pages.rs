//! The pages of the file-by-file view (RFC 005 part 6, handoff 2 of 2): the changes page of a revision, the page of one file, and a plain page for
//! every reason the view can be unavailable.
//!
//! The engine and the JSON side are `crate::changes`; these pages ask the same functions (the same cache, the same admission, the same bound on
//! waiters) and turn the answer into words. **Every name and every line of text reaches the page twice-escaped**: the engine's display function
//! has made it display-safe (controls, bidirectional controls, invalid bytes shown as visible escapes), and the template escapes it as HTML. Diff
//! text is text, never HTML: the sanitizer is not used on it. Nothing here is signed, selected or merged, and no signature outcome is shown.

use std::sync::Arc;

use askama::Template;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;

use planeter_store::{RepositoryRecord, RevisionRecord};
use planeter_transport::changes::display::is_disguise;
use planeter_transport::changes::model::{
    ChangeSet, DiffOp, FileDiff, FileEntry, Kind, NotShown, Status,
};
use planeter_transport::{ChangesReason, ChangesService, ProposalError, record_digest};

use super::proposal_pages::{nav_for, off_page, permissions, service_of};
use super::{Ctx, Who, ctx, error_page, pct, render, short};
use crate::api::{AppState, Failure};
use crate::catalog::{Key, Rich, T};
use crate::changes::{Fail, fetch_file_and_list, fetch_list};
use crate::gate::offload;
use crate::transport::readable;

/// Files per page of the list.
const PAGE: usize = crate::changes::LIST_DEFAULT;

// ------------------------------------------------------------------------------------------------ addresses

fn proposal_href(owner: &str, name: &str, id: &str) -> String {
    format!("/{owner}/{name}/proposals/{}", pct(id))
}

pub(super) fn changes_href(owner: &str, name: &str, id: &str, seq: u32) -> String {
    format!("{}/revisions/{seq}/changes", proposal_href(owner, name, id))
}

fn file_href(owner: &str, name: &str, id: &str, seq: u32, file_id: &str) -> String {
    format!("{}/{file_id}", changes_href(owner, name, id, seq))
}

// ------------------------------------------------------------------------------------------------ what the pages share

/// What every page of this module starts from: the repository, the proposal's revision and the engine.
struct Opened {
    record: RepositoryRecord,
    svc: Arc<ChangesService>,
    rev: RevisionRecord,
    /// The proposal's own id (as stored), for addresses.
    id: String,
    /// The download of this revision's files, when the viewer may and they are kept.
    download: Option<String>,
    proposals_nav: bool,
}

/// The read gate first (a hidden repository, an unknown proposal and an unknown revision are the same page), then the service, the revision and
/// the permission to download.
async fn open_page(
    state: &AppState,
    c: &Ctx,
    owner: &str,
    name: &str,
    id: &str,
    seq: &str,
) -> Result<Opened, Box<Response>> {
    let at = format!("/{owner}/{name}/proposals/{}", short(id));
    let not_found = || {
        Box::new(error_page(
            state,
            c,
            &at,
            Failure::Read(planeter_core::ReadError::NotFound),
        ))
    };
    let record = match readable(state, &c.principal, owner, name).await {
        Ok(r) => r,
        Err(e) => return Err(Box::new(error_page(state, c, &at, e))),
    };
    let Some(proposals) = service_of(state) else {
        return Err(Box::new(off_page(state, c, owner, name)));
    };
    let Some(svc) = state.transport.as_ref().and_then(|rt| rt.changes.clone()) else {
        return Err(Box::new(off_page(state, c, owner, name)));
    };
    let Ok(seq) = seq.parse::<u32>() else {
        return Err(not_found());
    };
    let found = {
        let (proposals, record, id) = (Arc::clone(&proposals), record.clone(), id.to_owned());
        offload(move || proposals.revision(&record, &id, seq)).await
    };
    let (proposal, rev) = match found {
        Ok(Ok((p, Some(r)))) => (p, r),
        Ok(Ok((_, None)) | Err(ProposalError::NotFound)) => return Err(not_found()),
        Ok(Err(_)) => return Err(Box::new(error_page(state, c, &at, Failure::Internal))),
        Err(_) => return Err(Box::new(error_page(state, c, &at, Failure::Busy))),
    };
    let (may_download, _) = permissions(state, c, &record, &proposal.opened_by).await;
    let kept =
        record_digest(&rev.artifact_digest).is_ok_and(|d| proposals.artifacts().contains(&d));
    let download = (may_download && kept).then(|| {
        format!(
            "/api/v1/repos/{owner}/{name}/proposals/{}/revisions/{}/artifact",
            pct(&proposal.change_id),
            rev.seq
        )
    });
    let (proposals_nav, _) = nav_for(state, c, owner, name).await;
    Ok(Opened {
        record,
        svc,
        rev,
        id: proposal.change_id,
        download,
        proposals_nav,
    })
}

/// The plain page of a reason the view is unavailable: a heading, what happened and what to do next, a way back (and the download when the viewer
/// may use it). The status is the engine's; `busy` carries `Retry-After`.
#[derive(Template)]
#[template(path = "changes_unavailable.html")]
struct UnavailableTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    owner: String,
    name: String,
    proposals_nav: bool,
    title: String,
    heading: String,
    body: String,
    seq: u32,
    proposal_href: String,
    download: Option<String>,
}

fn reason_keys(r: ChangesReason) -> (Key, Key, StatusCode) {
    match r {
        ChangesReason::ClaimNotHeld => (
            Key::ChangesUnavailableClaimNotHeldHeading,
            Key::ChangesUnavailableClaimNotHeldBody,
            StatusCode::CONFLICT,
        ),
        ChangesReason::NotOneChange => (
            Key::ChangesUnavailableNotOneChangeHeading,
            Key::ChangesUnavailableNotOneChangeBody,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ChangesReason::TreeTooLarge => (
            Key::ChangesUnavailableTreeTooLargeHeading,
            Key::ChangesUnavailableTreeTooLargeBody,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ChangesReason::Busy => (
            Key::ChangesUnavailableBusyHeading,
            Key::ChangesUnavailableBusyBody,
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        ChangesReason::TimedOut => (
            Key::ChangesUnavailableTimedOutHeading,
            Key::ChangesUnavailableTimedOutBody,
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        ChangesReason::Failed => (
            Key::ChangesUnavailableFailedHeading,
            Key::ChangesUnavailableFailedBody,
            StatusCode::SERVICE_UNAVAILABLE,
        ),
    }
}

/// The page for a failure of the engine.
fn fail_page(state: &AppState, c: &Ctx, o: &Opened, owner: &str, name: &str, f: Fail) -> Response {
    let at = format!("/{owner}/{name}/proposals/{}", short(&o.id));
    match f {
        Fail::NoSuchFile => error_page(
            state,
            c,
            &at,
            Failure::Read(planeter_core::ReadError::NotFound),
        ),
        Fail::OverBudget => error_page(state, c, &at, Failure::OverBudget),
        Fail::Internal => error_page(state, c, &at, Failure::Internal),
        Fail::Reason(r) => {
            let (heading_key, body_key, status) = reason_keys(r);
            let heading = c.t.get(heading_key);
            let tpl = UnavailableTpl {
                user: c.user.clone(),
                csrf: c.csrf.clone(),
                t: c.t,
                owner: owner.to_owned(),
                name: name.to_owned(),
                proposals_nav: o.proposals_nav,
                title: c
                    .t
                    .msg(Key::TitleChangesUnavailable)
                    .with("heading", &heading)
                    .with("seq", o.rev.seq)
                    .with("id", short(&o.id))
                    .with("repo", format!("{owner}/{name}"))
                    .product()
                    .text(),
                heading,
                body: c.t.get(body_key),
                seq: o.rev.seq,
                proposal_href: proposal_href(owner, name, &o.id),
                download: o.download.clone(),
            };
            let mut resp = render(state, status, &tpl, c);
            if r == ChangesReason::Busy
                && let Ok(v) = crate::api::retry_after_secs(state).to_string().parse()
            {
                resp.headers_mut()
                    .insert(axum::http::header::RETRY_AFTER, v);
            }
            resp
        }
    }
}

fn status_key(s: Status) -> Key {
    match s {
        Status::Added => Key::ChangesStatusAdded,
        Status::Modified => Key::ChangesStatusModified,
        Status::Deleted => Key::ChangesStatusDeleted,
        Status::TypeChanged => Key::ChangesStatusTypeChanged,
    }
}

fn note_key(n: NotShown) -> Key {
    match n {
        NotShown::TooLarge => Key::ChangesNoteTooLarge,
        NotShown::NotUtf8 => Key::ChangesNoteNotUtf8,
        NotShown::Binary => Key::ChangesNoteBinary,
        NotShown::Symlink => Key::ChangesNoteSymlink,
        NotShown::Directory => Key::ChangesNoteDirectory,
        NotShown::Unreadable => Key::ChangesNoteUnreadable,
    }
}

/// The page's own second line of defence (the first is the engine's display function, the second the template's HTML escaping; this is a third
/// look at what the engine returned, so that a name or a line that somehow reached here unescaped still cannot reorder or hide anything): every
/// control, separator, bidirectional or zero-width character becomes a visible `\u{…}` escape. In text a tab is kept and a carriage return is shown
/// as `␍`; in a name neither is kept. It is **idempotent** on what the engine already escaped (a backslash is not touched) and never cuts (a cut is the engine's, with its number beside the text).
fn safe(text: &str, in_text: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if in_text && c == '\t' {
            out.push('\t');
        } else if in_text && c == '\r' {
            out.push('\u{240D}');
        } else if is_disguise(c) {
            out.push_str(&format!("\\u{{{:X}}}", u32::from(c)));
        } else {
            out.push(c);
        }
    }
    out
}

/// The title of a page that names a file: the display path, cut so that a title stays a title.
fn title_path(path: &str) -> String {
    const MAX: usize = 80;
    if path.chars().count() > MAX {
        path.chars().take(MAX).collect::<String>() + "…"
    } else {
        path.to_owned()
    }
}

// ------------------------------------------------------------------------------------------------ the changes page

struct FileRow {
    path: String,
    /// "…(N more bytes of the name not shown)" when the name was cut.
    path_cut: Option<String>,
    /// The address of the file's page, when a diff is shown.
    href: Option<String>,
    status: String,
    /// "+3 −1" (for the eye; the words are for assistive technology).
    lines: Option<String>,
    lines_words: String,
    note: Option<String>,
}

#[derive(Template)]
#[template(path = "changes.html")]
struct ChangesTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    owner: String,
    name: String,
    proposals_nav: bool,
    title: String,
    heading: String,
    statements: Vec<Rich>,
    ids: Vec<Rich>,
    empty: bool,
    summary: String,
    unshown: Option<String>,
    caption: String,
    files: Vec<FileRow>,
    earlier: Option<String>,
    more: Option<String>,
    seq: u32,
    proposal_href: String,
    download: Option<String>,
}

/// The statements a reviewer reads before the table, in the order of the handoff: what the files are, how far to trust them, and what they are
/// compared with. Each is a plain sentence; the block ids are short forms in code (the full ids are under one `<details>`).
fn statements(t: &T, set: &ChangeSet) -> Vec<Rich> {
    let c = &set.compared;
    let mut out = vec![t.msg(Key::ChangesStatementRebuilt).html()];
    if set.provisional {
        out.push(t.msg(Key::ChangesStatementProvisional).html());
    }
    match (&c.before_block, &c.declared_base, c.base_differs) {
        (Some(real), Some(declared), true) => out.push(
            t.msg(Key::ChangesStatementBaseDiffers)
                .code("declared", short(declared))
                .code("real", short(real))
                .html(),
        ),
        (None, Some(declared), true) => out.push(
            t.msg(Key::ChangesStatementBaseDiffersNone)
                .code("declared", short(declared))
                .html(),
        ),
        (None, _, _) => out.push(t.msg(Key::ChangesStatementStarts).html()),
        _ => {}
    }
    if let Some(before) = &c.before_block {
        out.push(
            t.msg(Key::ChangesStatementCompared)
                .code("before", short(before))
                .html(),
        );
    }
    out
}

fn ids(t: &T, set: &ChangeSet) -> Vec<Rich> {
    let c = &set.compared;
    let mut out = Vec::new();
    if let Some(before) = &c.before_block {
        out.push(t.msg(Key::ChangesIdsBefore).code("id", before).html());
    }
    out.push(
        t.msg(Key::ChangesIdsAfter)
            .code("id", &c.after_block)
            .html(),
    );
    out
}

fn summary_line(t: &T, set: &ChangeSet) -> String {
    let n = &set.totals;
    let files = t.msg(Key::ChangesSummaryFiles).n(n.files_changed).text();
    let added = t.msg(Key::ChangesSummaryAdded).n(n.lines_added).text();
    let removed = t.msg(Key::ChangesSummaryRemoved).n(n.lines_removed).text();
    t.msg(Key::ChangesSummary)
        .with("files", files)
        .with("added", added)
        .with("removed", removed)
        .text()
}

/// The page's own words for a name the engine cut: the engine gives the head and the number, nothing else.
fn path_cut(t: &T, bytes: u64) -> Option<String> {
    (bytes > 0).then(|| t.msg(Key::ChangesCutBytes).n(bytes).text())
}

fn file_row(t: &T, o: &Opened, owner: &str, name: &str, f: &FileEntry) -> FileRow {
    let counted = f.shown.then_some((f.lines_added, f.lines_removed));
    let (lines, lines_words) = match counted {
        Some((Some(a), Some(r))) => (
            Some(format!("+{a} \u{2212}{r}")),
            t.msg(Key::ChangesRowLines)
                .with("added", a)
                .with("removed", r)
                .text(),
        ),
        _ => (None, t.get(Key::ChangesRowLinesNone)),
    };
    let note = match (f.not_shown_because, f.kind, f.shown) {
        (Some(n), _, _) => Some(t.get(note_key(n))),
        (None, Kind::Symlink, true) => Some(t.get(Key::ChangesNoteLink)),
        _ => None,
    };
    FileRow {
        path: safe(&f.path, false),
        path_cut: path_cut(t, f.path_cut_bytes),
        href: f
            .shown
            .then(|| file_href(owner, name, &o.id, o.rev.seq, &f.file_id)),
        status: t.get(status_key(f.status)),
        lines,
        lines_words,
        note,
    }
}

#[derive(serde::Deserialize)]
pub(super) struct ChangesQuery {
    after: Option<String>,
}

pub(super) async fn changes_page(
    State(state): State<AppState>,
    Path((owner, name, id, seq)): Path<(String, String, String, String)>,
    Query(q): Query<ChangesQuery>,
    headers: HeaderMap,
) -> Response {
    let c = ctx(&headers, &state).await;
    let o = match open_page(&state, &c, &owner, &name, &id, &seq).await {
        Ok(o) => o,
        Err(r) => return *r,
    };
    let set = match fetch_list(&state, &o.svc, &o.record, &o.rev).await {
        Ok(s) => s,
        Err(f) => return fail_page(&state, &c, &o, &owner, &name, f),
    };
    let t = c.t;
    let seq = o.rev.seq;
    // the page: the files after the cursor (an unknown cursor is the first page), and the cursor of the page before it
    let start = q
        .after
        .as_deref()
        .and_then(|a| set.files.iter().position(|f| f.file_id == a))
        .map_or(0, |i| i + 1);
    let end = (start + PAGE).min(set.files.len());
    let base = changes_href(&owner, &name, &o.id, seq);
    let earlier = (start > 0).then(|| {
        let prev = start.saturating_sub(PAGE);
        if prev == 0 {
            base.clone()
        } else {
            format!("{base}?after={}", set.files[prev - 1].file_id)
        }
    });
    let more =
        (end < set.files.len()).then(|| format!("{base}?after={}", set.files[end - 1].file_id));
    let files: Vec<FileRow> = set.files[start..end]
        .iter()
        .map(|f| file_row(&t, &o, &owner, &name, f))
        .collect();
    let heading = t
        .msg(Key::ChangesHeading)
        .with("seq", seq)
        .with("id", short(&o.id))
        .text();
    let repo = format!("{owner}/{name}");
    let tpl = ChangesTpl {
        user: c.user.clone(),
        csrf: c.csrf.clone(),
        t,
        title: t
            .msg(Key::TitleChanges)
            .with("seq", seq)
            .with("id", short(&o.id))
            .with("repo", &repo)
            .product()
            .text(),
        heading,
        statements: statements(&t, &set),
        ids: ids(&t, &set),
        empty: set.files.is_empty(),
        summary: summary_line(&t, &set),
        unshown: (set.totals.unshown > 0).then(|| {
            t.msg(Key::ChangesSummaryUnshown)
                .with("number", set.totals.unshown)
                .text()
        }),
        caption: t.msg(Key::ChangesTableCaption).with("seq", seq).text(),
        files,
        earlier,
        more,
        seq,
        proposal_href: proposal_href(&owner, &name, &o.id),
        download: o.download.clone(),
        owner,
        name,
        proposals_nav: o.proposals_nav,
    };
    render(&state, StatusCode::OK, &tpl, &c)
}

// ------------------------------------------------------------------------------------------------ the page of one file

struct DiffRow {
    class: &'static str,
    old: String,
    new: String,
    /// "+", "−" or nothing: for the eye; `word` is for assistive technology.
    marker: &'static str,
    word: String,
    text: String,
    /// "…(N more characters not shown)" when the line was cut.
    cut: Option<String>,
}

struct HunkView {
    header: String,
    lines: Vec<DiffRow>,
}

#[derive(Template)]
#[template(path = "change_file.html")]
struct FileTpl {
    user: Option<Who>,
    csrf: String,
    t: T,
    owner: String,
    name: String,
    proposals_nav: bool,
    title: String,
    path: String,
    path_cut: Option<String>,
    path_escaped: bool,
    status_line: String,
    /// A symbolic link: one sentence in place of the table.
    link_sentence: Option<Rich>,
    /// A file with no diff: the reason, in one sentence.
    not_shown: Option<String>,
    caption: Rich,
    hunks: Vec<HunkView>,
    /// The sentences under the table: lines left out, an approximate diff, cut lines.
    notes: Vec<String>,
    seq: u32,
    download: Option<String>,
    prev: Option<String>,
    next: Option<String>,
    list_href: String,
    proposal_href: String,
}

/// "Lines 3–5 before" / "Line 3 before" / "No lines before" (and after), joined.
fn hunk_header(t: &T, h: &planeter_transport::changes::model::DiffHunk) -> String {
    let side = |lines: u64, start: u64, one: Key, many: Key, none: Key| -> String {
        match lines {
            0 => t.get(none),
            1 => t.msg(one).with("from", start).text(),
            n => t
                .msg(many)
                .with("from", start)
                .with("to", start + n - 1)
                .text(),
        }
    };
    t.msg(Key::ChangesHunk)
        .with(
            "before",
            side(
                h.old_lines,
                h.old_start,
                Key::ChangesHunkBeforeOne,
                Key::ChangesHunkBeforeMany,
                Key::ChangesHunkBeforeNone,
            ),
        )
        .with(
            "after",
            side(
                h.new_lines,
                h.new_start,
                Key::ChangesHunkAfterOne,
                Key::ChangesHunkAfterMany,
                Key::ChangesHunkAfterNone,
            ),
        )
        .text()
}

fn hunk_views(t: &T, d: &FileDiff) -> Vec<HunkView> {
    d.hunks
        .iter()
        .map(|h| {
            let (mut old_no, mut new_no) = (
                if h.old_lines == 0 {
                    h.old_start + 1
                } else {
                    h.old_start
                },
                if h.new_lines == 0 {
                    h.new_start + 1
                } else {
                    h.new_start
                },
            );
            let lines = h
                .lines
                .iter()
                .map(|l| {
                    let (class, marker, word, old, new) = match l.op {
                        DiffOp::Context => {
                            let r = (
                                "diff-context",
                                "",
                                t.get(Key::ChangesRowSame),
                                old_no.to_string(),
                                new_no.to_string(),
                            );
                            old_no += 1;
                            new_no += 1;
                            r
                        }
                        DiffOp::Remove => {
                            let r = (
                                "diff-remove",
                                "\u{2212}",
                                t.get(Key::ChangesRowRemoved),
                                old_no.to_string(),
                                String::new(),
                            );
                            old_no += 1;
                            r
                        }
                        DiffOp::Add => {
                            let r = (
                                "diff-add",
                                "+",
                                t.get(Key::ChangesRowAdded),
                                String::new(),
                                new_no.to_string(),
                            );
                            new_no += 1;
                            r
                        }
                    };
                    DiffRow {
                        class,
                        old,
                        new,
                        marker,
                        word,
                        text: safe(&l.text, true),
                        cut: (l.cut_chars > 0)
                            .then(|| t.msg(Key::ChangesCutChars).n(l.cut_chars).text()),
                    }
                })
                .collect();
            HunkView {
                header: hunk_header(t, h),
                lines,
            }
        })
        .collect()
}

/// The sentence of a symbolic link, from the one or two lines the engine returned (the targets, display-safe, never a link).
fn link_sentence(t: &T, d: &FileDiff) -> Option<Rich> {
    let lines: Vec<(DiffOp, &str)> = d
        .hunks
        .iter()
        .flat_map(|h| h.lines.iter().map(|l| (l.op, l.text.as_str())))
        .collect();
    let of = |op: DiffOp| lines.iter().find(|(o, _)| *o == op).map(|(_, s)| *s);
    match (d.entry.status, of(DiffOp::Add), of(DiffOp::Remove)) {
        (Status::Modified, Some(new), Some(old)) => Some(
            t.msg(Key::ChangesLinkModified)
                .code("target", safe(new, true))
                .code("old", safe(old, true))
                .html(),
        ),
        (Status::Added, Some(target), _) => {
            Some(t.msg(Key::ChangesLinkAdded).code("target", target).html())
        }
        (Status::Deleted, _, Some(target)) => {
            Some(t.msg(Key::ChangesLinkDeleted).code("target", target).html())
        }
        _ => None,
    }
}

pub(super) async fn file_page(
    State(state): State<AppState>,
    Path((owner, name, id, seq, file_id)): Path<(String, String, String, String, String)>,
    headers: HeaderMap,
) -> Response {
    let c = ctx(&headers, &state).await;
    let o = match open_page(&state, &c, &owner, &name, &id, &seq).await {
        Ok(o) => o,
        Err(r) => return *r,
    };
    let at = format!("/{owner}/{name}/proposals/{}", short(&o.id));
    // a file id is 64 lowercase hexadecimal characters; anything else names nothing
    if file_id.len() != 64
        || !file_id
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return error_page(
            &state,
            &c,
            &at,
            Failure::Read(planeter_core::ReadError::NotFound),
        );
    }
    let (diff, set) = match fetch_file_and_list(&state, &o.svc, &o.record, &o.rev, &file_id).await {
        Ok(x) => x,
        Err(f) => return fail_page(&state, &c, &o, &owner, &name, f),
    };
    let t = c.t;
    let seq = o.rev.seq;
    let e = &diff.entry;
    let here = set.files.iter().position(|f| f.file_id == file_id);
    let neighbour = |i: Option<usize>| {
        i.and_then(|i| set.files.get(i))
            .filter(|f| f.shown)
            .map(|f| file_href(&owner, &name, &o.id, seq, &f.file_id))
    };
    // the previous / next changed file *whose diff is shown*, in list order
    let prev = here.and_then(|i| {
        (0..i)
            .rev()
            .find(|j| set.files[*j].shown)
            .and_then(|j| neighbour(Some(j)))
    });
    let next = here.and_then(|i| {
        (i + 1..set.files.len())
            .find(|j| set.files[*j].shown)
            .and_then(|j| neighbour(Some(j)))
    });
    let is_link = e.kind == Kind::Symlink && e.shown;
    let mut notes = Vec::new();
    if let Some(tr) = &diff.truncated {
        notes.push(
            t.msg(Key::ChangesFileTruncated)
                .n(tr.lines_not_shown)
                .text(),
        );
    }
    if diff.approximate {
        notes.push(t.get(Key::ChangesFileApproximate));
    }
    if diff
        .hunks
        .iter()
        .any(|h| h.lines.iter().any(|l| l.cut_chars > 0))
        && !is_link
    {
        notes.push(t.get(Key::ChangesFileCutLines));
    }
    let repo = format!("{owner}/{name}");
    let tpl = FileTpl {
        user: c.user.clone(),
        csrf: c.csrf.clone(),
        t,
        title: t
            .msg(Key::TitleChangesFile)
            .with("path", title_path(&safe(&e.path, false)))
            .with("seq", seq)
            .with("repo", &repo)
            .product()
            .text(),
        path: safe(&e.path, false),
        path_cut: path_cut(&t, e.path_cut_bytes),
        path_escaped: e.path_escaped,
        status_line: t
            .msg(Key::ChangesFileStatus)
            .with("status", t.get(status_key(e.status)))
            .text(),
        link_sentence: if is_link {
            link_sentence(&t, &diff)
        } else {
            None
        },
        not_shown: e.not_shown_because.map(|n| t.get(note_key(n))),
        caption: t
            .msg(Key::ChangesFileCaption)
            .code("path", safe(&e.path, false))
            .html(),
        hunks: if is_link {
            Vec::new()
        } else {
            hunk_views(&t, &diff)
        },
        notes,
        seq,
        download: if e.shown { None } else { o.download.clone() },
        prev,
        next,
        list_href: changes_href(&owner, &name, &o.id, seq),
        proposal_href: proposal_href(&owner, &name, &o.id),
        owner,
        name,
        proposals_nav: o.proposals_nav,
    };
    render(&state, StatusCode::OK, &tpl, &c)
}
