//! `planeter-client deliver`: a maintainer's own repository, exported with the maintainer's own prikk, sent to the forge to be checked,
//! and — only when the maintainer confirms — made the project's branch (RFC 008 D-1, D-3, D-5, D-9).
//!
//! The client signs nothing and merges nothing. It asks the server three things (`POST …/deliveries`, `POST …/deliveries/{id}/confirm`,
//! `DELETE …/deliveries/{id}`) and says what the server answered in words a maintainer reads: the server's own `message` and `hint`
//! (in the language the client asked for), then, from the report's fields, the numbers and the proposals the delivery touches.
//!
//! What is **not** here, on purpose: no call of prikk other than `archive export` and the reads `prikk.rs` already makes (the module's
//! source is scanned by `tests/deliver.rs` for `seal`, `merge`, `branch close`, `--allow-non-fast-forward` and a `--yes` to prikk); no retry
//! of a refused answer; no redirect followed; no token in any line printed.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::Ctx;
use crate::catalog::{Key, size as show_size, t};
use crate::cli::DeliverArgs;
use crate::commands::{client, local, parse_url, progress_line};
use crate::exit::{Failure, NETWORK, REFUSED, USAGE};
use crate::http::{Answer, Http, HttpError, Posted};
use crate::prikk::Prikk;
use crate::url::RepoUrl;

/// The framing of an upload, as the import's: the magic, an 8-byte big-endian length of the metadata, the metadata (`{}`), the archive.
const MAGIC: &[u8] = b"PLANETER-IMPORT-1\n";
/// The branch a delivery replaces (the server's `FOLLOWED_REF`).
const PROJECT_BRANCH: &str = "heads/main";
/// The prefix of this command's private scratch directories and of their lock files.
const SCRATCH_PREFIX: &str = "planeter-client-deliver-";

// ------------------------------------------------------------------------------------------------ pacing

/// How often and how long the command looks at a delivery. The defaults are the handoff's (2 s, a line every 30 s, 15 minutes); the
/// environment variables are for the tests only.
#[derive(Debug, Clone, Copy)]
struct Pace {
    poll: Duration,
    still: Duration,
    limit: Duration,
}

fn pace(ctx: &Ctx<'_>) -> Pace {
    let ms = |name: &str, default: u64| {
        Duration::from_millis(
            (ctx.env)(name)
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(default),
        )
    };
    Pace {
        poll: ms("PLANETER_CLIENT_POLL_MS", 2_000),
        still: ms("PLANETER_CLIENT_STILL_MS", 30_000),
        limit: ms("PLANETER_CLIENT_WAIT_MS", 15 * 60 * 1_000),
    }
}

// ------------------------------------------------------------------------------------------------ the private scratch

/// A private directory for the exported archive, removed on every exit path.
///
/// * **Private:** the directory is made with mode `0700` (Unix), and the archive in it is set to `0600`; nothing else can read it.
/// * **Removed on a normal end, an error and a panic** by `Drop`.
/// * **Removed after a kill:** the run holds an exclusive advisory lock on `<name>.lock` next to its directory for as long as it lives;
///   the operating system releases it when the process dies, however it dies. The next `deliver` finds every `*.lock` it can lock itself
///   (nobody holds it), and removes that lock's directory. (A lock file younger than two seconds is left alone: its owner may not have
///   locked it yet.)
struct PrivateScratch {
    dir: PathBuf,
    lock_path: PathBuf,
    lock: Option<std::fs::File>,
}

impl PrivateScratch {
    fn new(ctx: &Ctx<'_>) -> Result<PrivateScratch, Failure> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let base = (ctx.env)("TMPDIR")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let failed = |path: &Path, e: std::io::Error| local(ctx, path, &e);
        std::fs::create_dir_all(&base).map_err(|e| failed(&base, e))?;
        sweep_stale(&base);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        loop {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let name = format!("{SCRATCH_PREFIX}{}-{nanos}-{n}", std::process::id());
            let lock_path = base.join(format!("{name}.lock"));
            let dir = base.join(&name);
            let lock = match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(failed(&lock_path, e)),
            };
            if let Err(e) = lock.lock() {
                let _ = std::fs::remove_file(&lock_path);
                return Err(failed(&lock_path, e));
            }
            match make_private_dir(&dir) {
                Ok(()) => {
                    return Ok(PrivateScratch {
                        dir,
                        lock_path,
                        lock: Some(lock),
                    });
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&lock_path);
                    if e.kind() == std::io::ErrorKind::AlreadyExists {
                        continue;
                    }
                    return Err(failed(&dir, e));
                }
            }
        }
    }

    fn archive(&self) -> PathBuf {
        self.dir.join("archive")
    }
}

impl Drop for PrivateScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        drop(self.lock.take());
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

#[cfg(unix)]
fn make_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(dir)
}

#[cfg(not(unix))]
fn make_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir(dir)
}

#[cfg(unix)]
fn make_private_file(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn make_private_file(_path: &Path) {}

/// Remove the scratch directories of runs that died: every `<prefix>*.lock` in `base` whose lock can be taken is nobody's.
fn sweep_stale(base: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name
            .strip_prefix(SCRATCH_PREFIX)
            .and_then(|r| r.strip_suffix(".lock"))
        else {
            continue;
        };
        let young = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| m.elapsed().ok())
            .is_none_or(|age| age < Duration::from_secs(2));
        if young {
            continue;
        }
        let Ok(file) = std::fs::OpenOptions::new().write(true).open(entry.path()) else {
            continue;
        };
        if file.try_lock().is_ok() {
            let _ = std::fs::remove_dir_all(base.join(format!("{SCRATCH_PREFIX}{stem}")));
            let _ = file.unlock();
            drop(file);
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

// ------------------------------------------------------------------------------------------------ what the server says

/// A delivery as the server shows it (`DeliveryView`), read as the client reads every document: by the fields it needs.
#[derive(Debug, Clone)]
struct View {
    id: String,
    state: String,
    message: String,
    hint: String,
    needs_recovery: bool,
    reason: Option<String>,
    report: Value,
}

impl View {
    fn parse(body: &[u8]) -> Option<View> {
        let v: Value = serde_json::from_slice(body).ok()?;
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
        Some(View {
            id: text("id")?,
            state: text("state")?,
            message: text("message").unwrap_or_default(),
            hint: text("hint").unwrap_or_default(),
            needs_recovery: v
                .get("needs_recovery")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            reason: v
                .get("failure")
                .and_then(|f| f.get("reason"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            report: v.get("report").cloned().unwrap_or(Value::Null),
        })
    }

    fn acceptable(&self) -> bool {
        self.state == "reported"
            && self.report.get("verdict").and_then(Value::as_str) == Some("acceptable")
    }

    /// Nothing more will happen to a delivery in one of these states (or it waits for the server's recovery).
    fn over(&self) -> bool {
        self.needs_recovery
            || matches!(
                self.state.as_str(),
                "done" | "failed" | "abandoned" | "expired"
            )
    }
}

fn bad_answer(ctx: &Ctx<'_>, what: &str) -> Failure {
    Failure::new(NETWORK, t(ctx.lang, Key::NetBadAnswer).text()).with_detail(what.to_owned())
}

/// A server text shown to a person: control characters (a terminal escape in a proposal's title, say) are made visible, not obeyed.
pub fn visible(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() {
            out.push_str(&format!("\\u{{{:x}}}", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

/// The first twelve characters of an id, for a line a person reads.
fn short(id: &str) -> String {
    id.chars().take(12).collect()
}

/// `#id "title"` for each `{id, title}` of a list in the report, visibly escaped, joined by `; `.
fn proposal_list(list: &Value) -> (usize, String) {
    let items = list.as_array().cloned().unwrap_or_default();
    let parts: Vec<String> = items
        .iter()
        .map(|p| {
            let id = p.get("id").and_then(Value::as_str).unwrap_or("?");
            let title = p.get("title").and_then(Value::as_str).unwrap_or("");
            format!("#{} \"{}\"", visible(id), visible(title))
        })
        .collect();
    (items.len(), parts.join("; "))
}

fn api_url(url: &RepoUrl, id: &str) -> String {
    url.api(&format!("deliveries/{}", id))
}

// ------------------------------------------------------------------------------------------------ the command

pub fn run(ctx: &mut Ctx<'_>, args: &DeliverArgs) -> Result<(), Failure> {
    let url = parse_url(ctx, &args.url)?;
    let repo_dir = match &args.repo {
        Some(d) => ctx.cwd.join(d),
        None => ctx.cwd.clone(),
    };
    // 1. Local checks, before anything is exported or sent.
    if !Prikk::is_repository(&repo_dir) {
        return Err(Failure::new(
            USAGE,
            t(ctx.lang, Key::UsageNoRepository)
                .with("dir", repo_dir.display())
                .text(),
        ));
    }
    // Nobody to ask and no `--yes`: said now, before a byte is exported or sent (a dry run asks nothing).
    if !args.dry_run && !args.yes && !ctx.interactive {
        return Err(Failure::new(
            USAGE,
            t(ctx.lang, Key::DeliverNoTerminal).text(),
        ));
    }
    let prikk = Prikk::in_ctx(ctx, &repo_dir);
    prikk.check()?;
    prikk
        .run(
            "read your branch",
            &[
                "log",
                "--format",
                "json",
                "--ref",
                PROJECT_BRANCH,
                "--limit",
                "1",
            ],
        )
        .map_err(|f| {
            Failure::new(
                USAGE,
                t(ctx.lang, Key::DeliverNoMain)
                    .with("ref", PROJECT_BRANCH)
                    .text(),
            )
            .with_detail(f.detail.unwrap_or(f.message))
        })?;
    let http = client(ctx, &url, &args.common, true)?.asking_in_language();
    let pace = pace(ctx);

    // 2. Export, into a private file that is removed on every way out.
    let scratch = PrivateScratch::new(ctx)?;
    let archive = scratch.archive();
    ctx.say(t(ctx.lang, Key::DeliverStepExport).text());
    let archive_arg = archive.to_string_lossy().into_owned();
    prikk.run_explained(
        "export your repository",
        &["archive", "export", &archive_arg],
        Key::DeliverExportFailed,
    )?;
    make_private_file(&archive);
    let size = std::fs::metadata(&archive)
        .map_err(|e| local(ctx, &archive, &e))?
        .len();
    ctx.say(
        t(ctx.lang, Key::DeliverExported)
            .with("size", show_size(size))
            .text(),
    );

    // 3. Upload, in the import's framing.
    ctx.say(
        t(ctx.lang, Key::DeliverStepSend)
            .with("size", show_size(size))
            .with("server", &url.host)
            .text(),
    );
    let mut prefix = Vec::with_capacity(MAGIC.len() + 8 + 2);
    prefix.extend_from_slice(MAGIC);
    prefix.extend_from_slice(&2u64.to_be_bytes());
    prefix.extend_from_slice(b"{}");
    let mut lines: Vec<(u64, u64)> = Vec::new();
    let posted = http.post_framed(
        &url.api("deliveries"),
        &prefix,
        &archive,
        &mut |s, total| lines.push((s, total)),
    );
    for (s, total) in lines {
        progress_line(ctx, Key::ProgressSent, s, Some(total));
    }
    let mut view = match posted {
        Ok(Posted { body, .. }) => View::parse(&body)
            .ok_or_else(|| bad_answer(ctx, "the delivery has no `id` or no `state`"))?,
        Err(HttpError::Server(a)) => return Err(refusal(ctx, &http, &a, Some(size))),
        Err(e) => {
            let mut f = http.failure(&e);
            f.message.push('\n');
            f.message
                .push_str(&t(ctx.lang, Key::DeliverMayHaveArrived).text());
            return Err(f);
        }
    };
    // The archive is on the server (or refused): the private copy has done its work.
    drop(scratch);

    // 3b. A check that takes long is waited for.
    if view.state == "staged" {
        ctx.say(
            t(ctx.lang, Key::DeliverStepChecking)
                .with("server", &url.host)
                .text(),
        );
        view = wait(ctx, &http, &url, view, pace, |v| {
            v.state != "staged" || v.needs_recovery
        })?;
    }

    // 4. The report.
    if view.state == "failed" || view.state == "abandoned" || view.state == "expired" {
        return report_refused(ctx, &view);
    }
    if !view.acceptable() {
        return Err(bad_answer(
            ctx,
            &format!("the delivery is `{}`", view.state),
        ));
    }
    present_report(ctx, &view);
    let id = view.id.clone();

    if args.dry_run {
        cancel(ctx, &http, &url, &id);
        ctx.say(t(ctx.lang, Key::DeliverDryRun).text());
        return Ok(());
    }

    // 5. Confirm: asked for, unless `--yes`.
    if !args.yes && !confirmed(ctx) {
        cancel(ctx, &http, &url, &id);
        ctx.say(t(ctx.lang, Key::DeliverDeclined).text());
        return Ok(());
    }
    ctx.say(t(ctx.lang, Key::DeliverStepConfirm).text());
    ctx.say(
        t(ctx.lang, Key::DeliverWatch)
            .with("id", &id)
            .with("url", api_url(&url, &id))
            .text(),
    );
    let answered = http.post_empty(&format!("{}/confirm", api_url(&url, &id)));
    let mut view = match answered {
        Ok(Posted { body, .. }) => View::parse(&body)
            .ok_or_else(|| bad_answer(ctx, "the confirmation has no `id` or no `state`"))?,
        Err(HttpError::Server(a)) => {
            return Err(confirm_refusal(ctx, &http, &url, &id, &a));
        }
        Err(e) => {
            let mut f = http.failure(&e);
            f.message.push('\n');
            f.message.push_str(
                &t(ctx.lang, Key::DeliverConfirmUnknown)
                    .with("url", api_url(&url, &id))
                    .text(),
            );
            return Err(f);
        }
    };
    if !view.over() {
        view = wait(ctx, &http, &url, view, pace, View::over)?;
    }

    // 6. The end.
    if view.needs_recovery {
        return Err(Failure::new(
            REFUSED,
            t(ctx.lang, Key::DeliverNeedsRecovery).text(),
        ));
    }
    if view.state == "done" {
        ctx.say(view.message.clone());
        if !view.hint.is_empty() {
            ctx.say(view.hint.clone());
        }
        let new = view
            .report
            .get("new_tip")
            .and_then(Value::as_str)
            .map(short)
            .unwrap_or_default();
        ctx.say(
            t(ctx.lang, Key::DeliverDone)
                .with("name", &url.name)
                .with("server", &url.host)
                .with("new", new)
                .text(),
        );
        return Ok(());
    }
    // `failed`, or a delivery that was given up or removed while it waited.
    Err(failure_of(ctx, &view))
}

// ------------------------------------------------------------------------------------------------ the pieces

/// Look at the delivery every `pace.poll` until `finished`, saying "still working" every `pace.still`, and give up after `pace.limit`.
fn wait(
    ctx: &mut Ctx<'_>,
    http: &Http,
    url: &RepoUrl,
    first: View,
    pace: Pace,
    finished: impl Fn(&View) -> bool,
) -> Result<View, Failure> {
    let id = first.id.clone();
    let started = Instant::now();
    let mut last_line = Instant::now();
    let mut view = first;
    while !finished(&view) {
        if started.elapsed() >= pace.limit {
            let minutes = (pace.limit.as_secs() / 60).max(1);
            return Err(Failure::new(
                NETWORK,
                t(ctx.lang, Key::DeliverWaitOver)
                    .with("minutes", minutes)
                    .with("url", api_url(url, &id))
                    .text(),
            ));
        }
        std::thread::sleep(pace.poll);
        match http.get_json(&api_url(url, &id)) {
            Ok(v) => {
                view = View::parse(v.to_string().as_bytes())
                    .ok_or_else(|| bad_answer(ctx, "the delivery has no `id` or no `state`"))?;
            }
            Err(e) => {
                let mut f = http.failure(&e);
                f.message.push('\n');
                f.message.push_str(
                    &t(ctx.lang, Key::DeliverConfirmUnknown)
                        .with("url", api_url(url, &id))
                        .text(),
                );
                return Err(f);
            }
        }
        if !finished(&view) && last_line.elapsed() >= pace.still {
            last_line = Instant::now();
            ctx.say(
                t(ctx.lang, Key::DeliverStillWorking)
                    .with("seconds", started.elapsed().as_secs())
                    .text(),
            );
        }
    }
    Ok(view)
}

/// "Deliver it? [y/N]": only `y` or `yes` is a yes; an empty line, end of input and everything else is no.
fn confirmed(ctx: &mut Ctx<'_>) -> bool {
    let prompt = t(ctx.lang, Key::DeliverPrompt).text();
    let _ = write!(ctx.out, "{prompt} ");
    let _ = ctx.out.flush();
    let mut line = String::new();
    if ctx.stdin.read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Ask the server to give up a delivery; a failure to do so is a warning, never an error (the server removes it after a day).
fn cancel(ctx: &mut Ctx<'_>, http: &Http, url: &RepoUrl, id: &str) {
    if http.delete(&api_url(url, id)).is_err() {
        let _ = writeln!(ctx.err, "{}", t(ctx.lang, Key::DeliverCancelFailed).text());
    }
}

/// What an acceptable report says, in the server's words first and then in the client's own.
fn present_report(ctx: &mut Ctx<'_>, view: &View) {
    // The server's sentence, which carries the owner's wording ("Delivering replaces this repository's branch … nothing is signed by this
    // server.") and what to do next.
    ctx.say(view.message.clone());
    if !view.hint.is_empty() {
        ctx.say(view.hint.clone());
    }
    let r = &view.report;
    let tip = |k: &str| r.get(k).and_then(Value::as_str).map(short);
    if let (Some(old), Some(new)) = (tip("old_tip"), tip("new_tip"))
        && old != new
    {
        ctx.say(
            t(ctx.lang, Key::DeliverReportMoves)
                .with("old", old)
                .with("new", new)
                .text(),
        );
    }
    if let Some(n) = r.get("new_blocks").and_then(Value::as_u64) {
        ctx.say(t(ctx.lang, Key::DeliverReportAdds).with("count", n).text());
    }
    let refs: Vec<String> = r
        .get("refs_added")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(visible).collect())
        .unwrap_or_default();
    if !refs.is_empty() {
        ctx.say(
            t(ctx.lang, Key::DeliverReportRefsAdded)
                .with("refs", refs.join(", "))
                .text(),
        );
    }
    for (field, key) in [
        ("proposals_stay", Key::DeliverReportStay),
        ("proposals_land", Key::DeliverReportLand),
        ("proposals_cannot_be_kept", Key::DeliverReportCannotKeep),
    ] {
        let (count, list) = proposal_list(r.get(field).unwrap_or(&Value::Null));
        if count > 0 {
            ctx.say(
                t(ctx.lang, key)
                    .with("count", count)
                    .with("list", list)
                    .text(),
            );
        }
    }
}

/// A refused report (a question answered, not an HTTP error) or a delivery that ended: `nothing_to_deliver` is exit 0.
fn report_refused(ctx: &mut Ctx<'_>, view: &View) -> Result<(), Failure> {
    if view.reason.as_deref() == Some("nothing_to_deliver") {
        ctx.say(
            t(ctx.lang, Key::DeliverNothing)
                .with("message", &view.message)
                .text(),
        );
        return Ok(());
    }
    let mut lines = vec![t(ctx.lang, Key::DeliverRefused).text()];
    lines.push(view.message.clone());
    if !view.hint.is_empty() {
        lines.push(view.hint.clone());
    }
    // `unadopted_key` names the keys that could deliver: they are in the report.
    if view.reason.as_deref() == Some("unadopted_key") {
        let keys = view
            .report
            .get("trusted_keys")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if !keys.is_empty() {
            lines.push(t(ctx.lang, Key::DeliverKeysHeader).text());
            for k in keys {
                lines.push(format!(
                    "  {}",
                    t(ctx.lang, Key::DeliverKeyLine)
                        .with(
                            "key_id",
                            visible(k.get("key_id").and_then(Value::as_str).unwrap_or("?")),
                        )
                        .with(
                            "fingerprint",
                            visible(k.get("fingerprint").and_then(Value::as_str).unwrap_or("")),
                        )
                        .text()
                ));
            }
        }
    }
    Err(Failure::new(REFUSED, lines.join("\n")).with_detail(format!(
        "state {}, reason {}",
        view.state,
        view.reason.as_deref().unwrap_or("-")
    )))
}

/// A delivery that did not go through after the confirmation: our one sentence, then the server's reason and what to do.
fn failure_of(ctx: &Ctx<'_>, view: &View) -> Failure {
    let mut lines = vec![t(ctx.lang, Key::DeliverFailed).text(), view.message.clone()];
    if !view.hint.is_empty() {
        lines.push(view.hint.clone());
    }
    Failure::new(REFUSED, lines.join("\n")).with_detail(format!(
        "state {}, reason {}",
        view.state,
        view.reason.as_deref().unwrap_or("-")
    ))
}

/// The sentence for a refusal of the upload: a `403` and a `413` in the client's own words, a `404` the one existence-neutral sentence
/// every command uses, the rest in the server's own words (`http::failure`).
fn refusal(ctx: &Ctx<'_>, http: &Http, a: &Answer, size: Option<u64>) -> Failure {
    let detail = a.error.as_ref().map(|e| e.verbatim(a.status));
    let with = |mut f: Failure| {
        if let Some(d) = &detail {
            f = f.with_detail(d.clone());
        }
        f
    };
    match a.status {
        403 => with(Failure::new(
            REFUSED,
            t(ctx.lang, Key::DeliverForbidden).text(),
        )),
        413 => with(Failure::new(
            REFUSED,
            t(ctx.lang, Key::DeliverTooLarge)
                .with("size", show_size(size.unwrap_or(0)))
                .text(),
        )),
        _ => http.failure(&HttpError::Server(Box::new(a.clone()))),
    }
}

/// The sentence for a refusal of the confirmation.
fn confirm_refusal(ctx: &mut Ctx<'_>, http: &Http, url: &RepoUrl, id: &str, a: &Answer) -> Failure {
    let code = a.error.as_ref().map(|e| e.code.as_str()).unwrap_or("");
    match (a.status, code) {
        (409, "delivery_busy") => {
            // The delivery stays `reported` on the server, and would block the next one: it is given up.
            cancel(ctx, http, url, id);
            Failure::new(REFUSED, t(ctx.lang, Key::DeliverBusy).text()).with_detail(
                a.error
                    .as_ref()
                    .map(|e| e.verbatim(a.status))
                    .unwrap_or_default(),
            )
        }
        (409, "delivery_not_confirmable") => Failure::new(
            REFUSED,
            t(ctx.lang, Key::DeliverNotConfirmable)
                .with("url", api_url(url, id))
                .text(),
        ),
        (410, _) => Failure::new(REFUSED, t(ctx.lang, Key::DeliverExpired).text()),
        (503, "needs_recovery") => {
            Failure::new(REFUSED, t(ctx.lang, Key::DeliverNeedsRecovery).text())
        }
        (403, _) => refusal(ctx, http, a, None),
        _ => http.failure(&HttpError::Server(Box::new(a.clone()))),
    }
}
