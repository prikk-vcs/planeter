//! The English catalog (RFC 012 D-3): the words the pages say today, as whole sentences with named
//! placeholders. Wording is unchanged except where a fragment had to become a sentence (listed in the
//! message-catalog review request). The order follows [`super::Key`].

use super::{Entry, msg, plural_msg};

pub const CATALOG: &[Entry] = &[
    // layout and navigation
    msg("layout.sign_in", "sign in"),
    msg("layout.sign_out", "sign out"),
    msg("layout.skip", "Skip to content"),
    msg("layout.nav_repository", "Repository"),
    msg("layout.footer", "{product} {version}"),
    msg("nav.code", "code"),
    msg("nav.history", "history"),
    msg("nav.refs", "refs"),
    msg("nav.verify", "verify"),
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
    msg("title.too_large", "Too large · {path} · {product}"),
    msg("title.repo", "{repo} · {product}"),
    msg("title.tree", "{prefix} · {repo} · {product}"),
    msg("title.file", "{path} · {repo} · {product}"),
    msg("title.history", "history · {repo} · {product}"),
    msg("title.change", "change {target} · {repo} · {product}"),
    msg("title.refs", "refs · {repo} · {product}"),
    msg("title.verify", "verify · {repo} · {product}"),
    // home
    msg(
        "home.intro",
        "A forge for prikk repositories. Browse a repository by its {address}.",
    ),
    msg("home.address_example", "owner/name"),
    msg("home.address_label", "Repository address"),
    msg(
        "home.address_error",
        "That is not a repository address. Write it as {address}.",
    ),
    msg("home.open", "Open"),
    // sign-in
    msg("login.heading", "Sign in"),
    msg("login.failed", "Sign-in failed."),
    msg("login.sso", "Sign in with SSO"),
    msg("login.user", "User"),
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
        "There is no such repository, or you do not have access to it.",
    ),
    msg("unavailable.heading", "Temporarily unavailable"),
    msg(
        "unavailable.body",
        "The repository could not be read right now. Nothing is shown rather than something stale or invented.",
    ),
    msg("busy.heading", "The server is busy"),
    msg(
        "busy.body",
        "Too many requests are being handled right now. Try again in a little while.",
    ),
    msg("too_large.heading", "Too large"),
    // repository, tree, file
    msg("repo.location", "at {point}"),
    msg("repo.location_branch", "at {point} · branch {branch}"),
    msg("tree.at", "at {point}"),
    msg("listing.size_and_encoding", "{size} · {encoding}"),
    msg("listing.caption", "Files and folders"),
    msg("listing.name", "Name"),
    msg("listing.details", "Size and type"),
    msg("listing.folder", "folder"),
    msg("file.binary", "Binary content — {download}."),
    msg("file.download_named", "Download {name}"),
    // sizes
    msg("size.bytes", "{value} B"),
    msg("size.kib", "{value} KiB"),
    msg("size.mib", "{value} MiB"),
    msg("size.gib", "{value} GiB"),
    // assurance labels (trust-bearing)
    msg("assurance.verified", "prikk-verified"),
    msg("assurance.queued", "forge-approved, unsealed"),
    msg("assurance.unverified", "unverified"),
    msg("assurance.verify_unavailable", "verify unavailable"),
    // history, change, refs, verify
    msg("history.heading", "History {ref}"),
    msg("history.rollback", "rollback"),
    plural_msg("history.patch_count", "{count} patch", "{count} patches"),
    msg("history.empty", "No sealed history on this ref yet."),
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
        "Re-derived from {command} on this request — never served from a cache.",
    ),
    plural_msg(
        "verify.sessions",
        "{verified} of {count} session verified.",
        "{verified} of {count} sessions verified.",
    ),
    msg("verify.status.evaluated", "evaluated"),
    msg("verify.status.failed", "failed"),
    msg("verify.status.not_evaluated", "not_evaluated"),
    msg("verify.status.halted", "halted"),
    msg("verify.col.stage", "Stage"),
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
        "This server does not serve files that large; ask the operator.",
    ),
    msg(
        "error.internal.message",
        "The server could not complete the request.",
    ),
    msg(
        "error.internal.hint",
        "Try again later; if it persists, tell the operator.",
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
        "The form could not be checked, so nothing was done.",
    ),
    msg(
        "page.form_incomplete.title",
        "The form was incomplete · {product}",
    ),
    msg("page.form_incomplete.heading", "The form was incomplete"),
    msg(
        "page.form_incomplete.body",
        "Some required fields were missing, so nothing was done.",
    ),
    msg(
        "page.sso_off.title",
        "Single sign-on is not set up · {product}",
    ),
    msg("page.sso_off.heading", "Single sign-on is not set up"),
    msg(
        "page.sso_off.body",
        "This server has no single sign-on provider configured.",
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
        "The sign-in provider could not be reached.",
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
        "The sign-in provider's reply was incomplete.",
    ),
    msg(
        "page.sso_mismatch.title",
        "The sign-in did not match · {product}",
    ),
    msg("page.sso_mismatch.heading", "The sign-in did not match"),
    msg(
        "page.sso_mismatch.body",
        "The sign-in could not be matched to the request that started it.",
    ),
    msg(
        "page.sso_expired.title",
        "The sign-in has expired · {product}",
    ),
    msg("page.sso_expired.heading", "The sign-in has expired"),
    msg(
        "page.sso_expired.body",
        "The sign-in took too long and has expired.",
    ),
    msg(
        "page.sso_refused.title",
        "The sign-in was not accepted · {product}",
    ),
    msg("page.sso_refused.heading", "The sign-in was not accepted"),
    msg(
        "page.sso_refused.body",
        "The sign-in provider did not accept the sign-in.",
    ),
    msg(
        "page.sso_unlinked.title",
        "This account is not linked · {product}",
    ),
    msg("page.sso_unlinked.heading", "This account is not linked"),
    msg(
        "page.sso_unlinked.body",
        "This single sign-on identity is not linked to a planeter account.",
    ),
    msg(
        "page.server_error.title",
        "Something went wrong · {product}",
    ),
    msg("page.server_error.heading", "Something went wrong"),
    msg(
        "page.server_error.body",
        "The server could not complete the sign-in.",
    ),
];
