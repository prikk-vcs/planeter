//! The English catalog (RFC 012 D-3): whole sentences with named placeholders. The wording follows
//! `docs/glossary.md` (RFC 012 D-4, plain words): a change to a user-facing message is a change to that page
//! first, and `tests/glossary.rs` fails if the two disagree. The order follows [`super::Key`].

use super::{Entry, msg, plural_msg};

pub const CATALOG: &[Entry] = &[
    // layout and navigation
    msg("layout.sign_in", "Sign in"),
    msg("layout.sign_out", "Sign out"),
    msg("layout.skip", "Skip to content"),
    msg("layout.nav_repository", "Repository"),
    msg("layout.footer", "{product} {version}"),
    msg("layout.language", "Language"),
    msg("nav.code", "Files"),
    msg("nav.history", "History"),
    msg("nav.refs", "Branches and tags"),
    msg("nav.verify", "Verification"),
    // page titles (one pattern: page · context · product)
    msg("title.home", "Find a repository · {product}"),
    msg(
        "title.home_error",
        "Find a repository · address not recognised · {product}",
    ),
    msg("title.busy", "Busy · {path} · {product}"),
    msg("title.login", "Sign in · {product}"),
    msg("title.login_failed", "Sign in · not accepted · {product}"),
    msg(
        "title.login_locked",
        "Sign in · too many attempts · {product}",
    ),
    msg("title.not_found", "Not found · {path} · {product}"),
    msg("title.unavailable", "Unavailable · {path} · {product}"),
    msg(
        "title.too_large",
        "Too large to show here · {path} · {product}",
    ),
    msg("title.repo", "{repo} · {product}"),
    msg("title.tree", "{prefix} · {repo} · {product}"),
    msg("title.file", "{path} · {repo} · {product}"),
    msg("title.history", "History · {repo} · {product}"),
    msg("title.change", "Change {target} · {repo} · {product}"),
    msg("title.refs", "Branches and tags · {repo} · {product}"),
    msg("title.verify", "Verification · {repo} · {product}"),
    // home
    msg(
        "home.intro",
        "planeter hosts prikk repositories. To open one, enter its address, written as {address}.",
    ),
    msg("home.address_example", "owner/name"),
    msg("home.address_label", "Repository address"),
    msg(
        "home.address_error",
        "That is not a repository address. Write it as {address}.",
    ),
    msg("home.open", "Open repository"),
    // sign-in
    msg("login.heading", "Sign in"),
    msg(
        "login.failed",
        "The username or password was not correct. Check both and try again.",
    ),
    msg("login.sso", "Sign in with single sign-on"),
    msg("login.user", "Username"),
    msg("login.password", "Password"),
    msg("login.submit", "Sign in"),
    msg(
        "login.locked",
        "Too many sign-in attempts. Wait a few minutes and try again.",
    ),
    // the plain pages
    msg("not_found.heading", "Not found"),
    msg(
        "not_found.body",
        "There is no repository at this address, or you do not have access to it. Check the address, or sign in if the repository is private.",
    ),
    msg("unavailable.heading", "Temporarily unavailable"),
    msg(
        "unavailable.body",
        "This repository could not be read right now. Try again in a little while. If it keeps happening, tell the person who runs this server.",
    ),
    msg("busy.heading", "The server is busy"),
    msg(
        "busy.body",
        "Too many requests are being handled right now. Try again in a little while.",
    ),
    msg("too_large.heading", "Too large to show here"),
    msg(
        "too_large.body",
        "This is too large for this server to show. You can download it, or ask the person who runs this server to raise the limit.",
    ),
    // a missing path in a repository the reader may read (IS-45); the heading and title are `not_found.*`
    msg(
        "no_such_path.body",
        "This file or folder was not found in this repository. Check the path, or go back to the files.",
    ),
    msg("no_such_path.link", "Go to the files"),
    // repository, tree, file
    msg("repo.location", "Showing {point}"),
    msg("repo.location_branch", "Showing {point} · branch {branch}"),
    msg("tree.at", "Showing {point}"),
    msg("listing.size_and_encoding", "{size} · {encoding}"),
    msg("listing.caption", "Files and folders"),
    msg("listing.name", "Name"),
    msg("listing.details", "Size and type"),
    msg("listing.folder", "folder"),
    msg(
        "file.binary",
        "This file is not text, so it cannot be shown here. {download}.",
    ),
    msg("file.download_named", "Download {name}"),
    // sizes
    msg("size.bytes", "{value} B"),
    msg("size.kib", "{value} KiB"),
    msg("size.mib", "{value} MiB"),
    msg("size.gib", "{value} GiB"),
    // assurance labels (trust-bearing)
    msg("assurance.verified", "Verified"),
    msg("assurance.sealed", "Signed into the history"),
    msg("assurance.queued", "Approved, awaiting signature"),
    msg("assurance.unverified", "Verification failed"),
    msg("assurance.verify_unavailable", "Could not be checked now"),
    // in-place explanations (a native <details>, no script); the wording is on the glossary page.
    msg("explain.summary", "What does this mean?"),
    msg(
        "explain.verified",
        "prikk ran all of its checks on this repository just now, and they found no problems.",
    ),
    msg(
        "explain.signed",
        "A maintainer signed this patch into the project's history. The Verification page shows whether the whole repository checks out.",
    ),
    msg(
        "explain.queued",
        "This change was approved on this server. A maintainer has not yet signed it into the project's history, so it is not part of the verified record.",
    ),
    msg(
        "explain.failed",
        "prikk checked this repository and found problems. The Verification page lists what was found.",
    ),
    msg(
        "explain.failed_here",
        "prikk checked this repository and found problems. They are listed below.",
    ),
    msg("explain.unavailable", "Try again in a little while."),
    msg("explain.patch", "A patch is one recorded edit."),
    msg(
        "explain.rollback",
        "A rollback is a change that undoes an earlier change.",
    ),
    // history, change, refs, verify
    msg("history.heading", "History {ref}"),
    msg("history.rollback", "rollback"),
    plural_msg("history.patch_count", "{count} patch", "{count} patches"),
    msg(
        "history.empty",
        "There are no signed changes on this branch yet.",
    ),
    msg("history.col.change", "Change"),
    msg("history.col.kind", "Kind"),
    msg("history.col.patches", "Patches"),
    msg("history.change_fallback", "Change {id}"),
    msg("change.heading", "Change {target}"),
    msg("change.patch", "patch {id}"),
    msg("change.col.operation", "Operation"),
    msg("change.col.paths", "Paths"),
    msg("refs.branches", "Branches"),
    msg("refs.tags", "Tags"),
    msg("refs.current", "current"),
    msg("refs.closed", "closed"),
    msg("refs.no_tags", "No tags."),
    msg("refs.heading", "Branches and tags"),
    msg("refs.col.branch", "Branch"),
    msg("refs.col.tag", "Tag"),
    msg("refs.col.state", "State"),
    msg("refs.col.target", "Target"),
    msg("verify.heading", "Verification"),
    msg(
        "verify.intro",
        "These checks were run just now, when you opened this page. They are never reused from an earlier visit.",
    ),
    msg("verify.findings", "What was found"),
    msg("verify.status.evaluated", "completed"),
    msg("verify.status.failed", "problem found"),
    msg(
        "verify.status.not_evaluated",
        "not run (blocked by {check})",
    ),
    // The status of a not-run check whose blocker prikk did not name (never seen from prikk 0.52).
    msg("verify.status.not_run", "not run"),
    msg("verify.status.halted", "stopped early"),
    msg(
        "verify.status.halted_after",
        "stopped early (after {check})",
    ),
    msg("verify.col.stage", "Check"),
    msg("verify.col.status", "Status"),
    msg("verify.col.detail", "Detail"),
    // the API's error messages and hints
    msg("error.not_found.message", "No such resource."),
    msg(
        "error.not_found.hint",
        "Check the owner, the repository name and the path, and that your token may read it.",
    ),
    msg(
        "error.unavailable.message",
        "The repository could not be read right now.",
    ),
    msg("error.unavailable.hint", "Try again in a little while."),
    msg("error.busy.message", "The server is busy."),
    msg(
        "error.busy.hint",
        "Retry after the number of seconds in the Retry-After header.",
    ),
    msg("error.invalid_request.message", "The request is malformed."),
    msg(
        "error.invalid_request.hint",
        "Check the request against the OpenAPI document at {document}.",
    ),
    msg(
        "error.method_not_allowed.message",
        "That method is not supported here.",
    ),
    msg("error.method_not_allowed.hint", "Use GET."),
    msg(
        "error.raw_elsewhere.message",
        "Raw file bytes are not served from this address.",
    ),
    msg(
        "error.raw_elsewhere.hint",
        "Use the link on the file's page, or the content origin named in the OpenAPI document.",
    ),
    msg(
        "error.too_large.message",
        "This is too large to serve here.",
    ),
    msg(
        "error.too_large.hint",
        "Download it from the content origin, or ask the operator to raise the read memory budget.",
    ),
    msg(
        "error.too_large.raw_hint",
        "This server does not serve files that large; ask the person who runs this server.",
    ),
    msg(
        "error.internal.message",
        "The server could not complete the request.",
    ),
    msg(
        "error.internal.hint",
        "Try again later; if it persists, tell the operator.",
    ),
    msg(
        "error.transport_off.message",
        "Artifact fetches are not set up on this server.",
    ),
    msg(
        "error.transport_off.hint",
        "Ask the person who runs this server to configure the export spool.",
    ),
    msg(
        "error.unknown_ref.message",
        "The repository has no ref with that name.",
    ),
    msg(
        "error.unknown_ref.hint",
        "Take the ref names from the advertisement of the repository.",
    ),
    msg(
        "error.unknown_kind.message",
        "That fetch kind is not supported.",
    ),
    msg(
        "error.unknown_kind.hint",
        "Use kind=archive or kind=bundle.",
    ),
    msg(
        "error.bundle_of_received_ref.message",
        "A received ref cannot be fetched as a bundle.",
    ),
    msg(
        "error.bundle_of_received_ref.hint",
        "Fetch the whole repository with kind=archive instead.",
    ),
    msg(
        "error.not_verified.message",
        "The exported artifact did not pass verification, so it was not served.",
    ),
    msg(
        "error.not_verified.hint",
        "Try again in a little while, or fetch a published ref as kind=bundle.",
    ),
    msg(
        "error.export_failed.message",
        "The artifact could not be exported right now.",
    ),
    msg("error.export_failed.hint", "Try again in a little while."),
    msg(
        "error.export_failed.deadline_hint",
        "The export took longer than this server allows; ask the person who runs this server to raise the export deadline.",
    ),
    msg(
        "error.export_failed.ceiling_hint",
        "The export needs more memory than this server allows; ask the person who runs this server to raise the export memory limit.",
    ),
    msg(
        "error.export_failed.spool_hint",
        "The artifact is larger than the whole export spool; ask the person who runs this server to enlarge it.",
    ),
    msg("error.rate_limited.message", "Too many requests."),
    msg(
        "error.rate_limited.hint",
        "Retry after the number of seconds in the Retry-After header.",
    ),
    msg(
        "error.range_not_satisfiable.message",
        "The requested byte range cannot be served.",
    ),
    msg(
        "error.range_not_satisfiable.hint",
        "Ask for a range inside the artifact; the Content-Range header gives its size.",
    ),
    msg(
        "error.invalid_request.ref_hint",
        "A bundle needs a ref: add ref=NAME, taking the name from the advertisement.",
    ),
    // quota refusals of a proposed change (RFC 005 D-9)
    msg(
        "quota.open_changes.message",
        "You already have {used} open changes in this repository, and the limit is {limit}.",
    ),
    msg(
        "quota.open_changes.hint",
        "Close one of your open changes, or ask a maintainer to raise the limit.",
    ),
    msg(
        "quota.accepted_bytes.message",
        "You have already sent {used} MiB to this repository, and the limit is {limit} MiB.",
    ),
    msg(
        "quota.accepted_bytes.hint",
        "Ask a maintainer to raise the limit, because what you have sent stays on the server even after a change is closed.",
    ),
    msg(
        "quota.revisions.message",
        "This change already has {used} revisions, and the limit is {limit}.",
    ),
    msg(
        "quota.revisions.hint",
        "Open a new change for further work, or ask a maintainer to raise the limit.",
    ),
    msg(
        "quota.capacity.message",
        "This repository has reached its capacity for proposed changes: {used} MiB of {limit} MiB are in use.",
    ),
    msg(
        "quota.capacity.hint",
        "Ask a maintainer to close finished changes, or ask the person who runs this server to raise the capacity.",
    ),
    // proposing a change (RFC 004 part 3b)
    msg(
        "error.forbidden.message",
        "You can read this repository, but you may not propose changes to it.",
    ),
    msg(
        "error.forbidden.hint",
        "Ask a maintainer for write access, or to open proposals to every signed-in user.",
    ),
    msg(
        "error.length_required.message",
        "The proposal did not say how long it is.",
    ),
    msg(
        "error.length_required.hint",
        "Send the proposal with a Content-Length header.",
    ),
    msg(
        "error.payload_too_large.message",
        "That proposal is larger than this server accepts.",
    ),
    msg(
        "error.payload_too_large.hint",
        "Send a smaller artifact, or ask the person who runs this server to raise the limit.",
    ),
    msg(
        "error.needs_recovery.message",
        "This repository is waiting for recovery after a write that did not finish, so it takes no new proposals right now.",
    ),
    msg(
        "error.needs_recovery.hint",
        "Try again later, because reading is not affected and the person who runs this server can see why in the server log.",
    ),
    msg(
        "error.quota_exceeded.message",
        "A limit on proposals to this repository was reached.",
    ),
    msg(
        "error.quota_exceeded.hint",
        "The details are in this response, so close a change or ask a maintainer to raise the limit.",
    ),
    msg(
        "error.base_unknown.message",
        "The base you named is not in this repository's history.",
    ),
    msg(
        "error.base_unknown.hint",
        "Send the base first, or leave it out.",
    ),
    msg(
        "error.declaration_misses_added.message",
        "Your list of patches leaves out patches that this artifact added.",
    ),
    msg(
        "error.declaration_misses_added.hint",
        "List every patch the artifact contains, then send it again.",
    ),
    msg(
        "error.declared_not_held.message",
        "Your list of patches names patches this repository does not hold.",
    ),
    msg(
        "error.declared_not_held.hint",
        "List only patches that are in the artifact or already in the repository.",
    ),
    msg(
        "error.no_new_content.message",
        "This artifact adds nothing that the repository did not already hold.",
    ),
    msg(
        "error.no_new_content.hint",
        "There is nothing to propose, so if you changed something, build the artifact again.",
    ),
    msg(
        "error.unknown_change.message",
        "This repository has no change with that id.",
    ),
    msg(
        "error.unknown_change.hint",
        "Take the id from an earlier response, or leave it out to open a new change.",
    ),
    msg(
        "error.change_not_open.message",
        "That change has landed or been closed, so it takes no more revisions.",
    ),
    msg("error.change_not_open.hint", "Open a new change instead."),
    msg(
        "error.not_your_change.message",
        "That change was opened by somebody else.",
    ),
    msg(
        "error.not_your_change.hint",
        "Open a change of your own, or ask a maintainer.",
    ),
    msg(
        "error.accept_refused.message",
        "prikk refused this artifact, and nothing was added to the repository.",
    ),
    msg(
        "error.accept_refused.hint",
        "Read the reason in the detail field, then fix the artifact and send it again.",
    ),
    msg(
        "error.intent_pending.message",
        "Somebody else has already started sending this exact artifact and it has not finished.",
    ),
    msg(
        "error.intent_pending.hint",
        "Try again once the repository has recovered.",
    ),
    msg(
        "error.incomplete.message",
        "The repository did not finish taking in your artifact.",
    ),
    msg(
        "error.incomplete.hint",
        "Send the same artifact again once the repository has recovered.",
    ),
    msg(
        "error.invalid_request.framing_hint",
        "Start the body with PLANETER-PROPOSE-1, then the metadata length, the metadata and the artifact, as the API document describes.",
    ),
    msg(
        "error.invalid_request.metadata_hint",
        "Fix the field named in the reason and send the proposal again, as the API document lists the fields.",
    ),
    msg(
        "error.propose_off.message",
        "Proposals are not set up on this server.",
    ),
    msg(
        "error.propose_off.hint",
        "Ask the person who runs this server to configure the artifact store.",
    ),
    msg(
        "propose.held",
        "Your work is held for review on this server, and no branch has changed.",
    ),
    msg(
        "propose.already_recorded",
        "This exact artifact was already recorded, so nothing was added, and no branch has changed.",
    ),
    // pages that used to be bare strings (T7): a heading, one sentence, one link
    msg("page.link.sign_in", "Go to the sign-in page"),
    msg("page.link.home", "Go to the home page"),
    msg("page.template_error", "template error"),
    msg(
        "page.csrf.title",
        "The request was not accepted · {product}",
    ),
    msg("page.csrf.heading", "The request was not accepted"),
    msg(
        "page.csrf.body",
        "The form could not be confirmed as yours, so nothing was done. Go back, reload the page and try again.",
    ),
    msg(
        "page.form_incomplete.title",
        "The form was incomplete · {product}",
    ),
    msg("page.form_incomplete.heading", "The form was incomplete"),
    msg(
        "page.form_incomplete.body",
        "Some fields were empty, so nothing was done. Go back and fill them in.",
    ),
    msg(
        "page.sso_off.title",
        "Single sign-on is not set up · {product}",
    ),
    msg("page.sso_off.heading", "Single sign-on is not set up"),
    msg(
        "page.sso_off.body",
        "This server has no single sign-on provider configured. Try signing in with your username instead.",
    ),
    msg(
        "page.sso_down.title",
        "Single sign-on is not available right now · {product}",
    ),
    msg(
        "page.sso_down.heading",
        "Single sign-on is not available right now",
    ),
    msg(
        "page.sso_down.body",
        "The sign-in provider could not be reached. Try signing in with your username instead.",
    ),
    msg(
        "page.sso_incomplete.title",
        "The sign-in did not complete · {product}",
    ),
    msg(
        "page.sso_incomplete.heading",
        "The sign-in did not complete",
    ),
    msg(
        "page.sso_incomplete.body",
        "The sign-in provider's reply was incomplete. Try signing in with your username instead.",
    ),
    msg(
        "page.sso_mismatch.title",
        "The sign-in did not match · {product}",
    ),
    msg("page.sso_mismatch.heading", "The sign-in did not match"),
    msg(
        "page.sso_mismatch.body",
        "The sign-in could not be matched to the request that started it. Try signing in with your username instead.",
    ),
    msg(
        "page.sso_expired.title",
        "The sign-in has expired · {product}",
    ),
    msg("page.sso_expired.heading", "The sign-in has expired"),
    msg(
        "page.sso_expired.body",
        "The sign-in took too long and has expired. Try signing in with your username instead.",
    ),
    msg(
        "page.sso_refused.title",
        "The sign-in was not accepted · {product}",
    ),
    msg("page.sso_refused.heading", "The sign-in was not accepted"),
    msg(
        "page.sso_refused.body",
        "The sign-in provider did not accept the sign-in. Try signing in with your username instead.",
    ),
    msg(
        "page.sso_unlinked.title",
        "This account is not linked · {product}",
    ),
    msg("page.sso_unlinked.heading", "This account is not linked"),
    msg(
        "page.sso_unlinked.body",
        "This single sign-on identity is not linked to a planeter account. Ask the person who runs this server.",
    ),
    msg(
        "page.server_error.title",
        "Something went wrong · {product}",
    ),
    msg("page.server_error.heading", "Something went wrong"),
    msg(
        "page.server_error.body",
        "Something went wrong on the server, and you were not signed in. Try again in a little while.",
    ),
];
