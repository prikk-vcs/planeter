//! `planeter-client propose`: ask the server what it holds, build an artifact against it with the user's own prikk, declare what
//! the artifact contains, send it in the propose framing, and say in plain words what happened and what comes next.
//!
//! Nothing here signs, merges or publishes: the server holds the work for review and no branch changes. A step that is not
//! idempotent is never retried silently; the re-send after an unfinished accept is the person's explicit `--retry`.

use crate::Ctx;
use crate::catalog::{Key, size as show_size, t};
use crate::cli::ProposeArgs;
use crate::commands::{Advert, Scratch, advertise, client, local, parse_url, progress_line};
use crate::exit::{Failure, LOCAL, NETWORK, REFUSED, USAGE};
use crate::http::{Answer, HttpError, Posted};
use crate::prikk::Prikk;
use crate::state;

/// The first bytes of a proposal's body (`docs/propose.md`).
const MAGIC: &[u8] = b"PLANETER-PROPOSE-1\n";
/// The most of the server's have-list that is taken.
const HAVE_MAX: u64 = 64 << 20;

/// Percent-encode a query value (a ref name keeps its `/`).
fn query_value(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// 64-bit FNV-1a, to tell "the same work" from other work in the local record of an unfinished send. (Not a security
/// measure: the record is the person's own.)
fn fnv(parts: &[&str]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for b in p.bytes().chain(std::iter::once(0xff)) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("{h:016x}")
}

/// The patch ids of the blocks of `git_ref` above `base`, newest first; `None` when `base` is not in the history.
fn patches_above(
    prikk: &Prikk,
    git_ref: &str,
    base: Option<&str>,
) -> Result<Option<Vec<String>>, Failure> {
    let mut limit = 256usize;
    loop {
        let v = prikk.json(
            "read your history",
            &[
                "log",
                "--format",
                "json",
                "--ref",
                git_ref,
                "--limit",
                &limit.to_string(),
            ],
        )?;
        let blocks = v
            .get("blocks")
            .and_then(|b| b.as_array())
            .cloned()
            .unwrap_or_default();
        let mut ids = Vec::new();
        for b in &blocks {
            if base.is_some_and(|base| b.get("block_id").and_then(|x| x.as_str()) == Some(base)) {
                return Ok(Some(ids));
            }
            if let Some(ps) = b.get("patch_messages").and_then(|p| p.as_array()) {
                for p in ps {
                    if let Some(id) = p.get("patch_id").and_then(|x| x.as_str()) {
                        ids.push(id.to_owned());
                    }
                }
            }
        }
        if blocks.len() < limit {
            // The whole history was read. With no base on the server, everything is new.
            return Ok(base.is_none().then_some(ids));
        }
        limit = limit.saturating_mul(4);
    }
}

fn number(v: &serde_json::Value, k: &str) -> String {
    match v.get(k) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(n) if n.is_number() => n.to_string(),
        _ => "?".to_owned(),
    }
}

/// The failure for a refusal of the propose route: one sentence for the reason, one for what to do; the server's own text only in
/// the detail.
fn refusal(ctx: &Ctx<'_>, http: &crate::http::Http, a: &Answer, retry_hint: bool) -> Failure {
    let code = a.error.as_ref().map(|e| e.code.as_str()).unwrap_or("");
    let raw = a.error.as_ref().map(|e| &e.raw);
    let l = ctx.lang;
    let message = match code {
        "incomplete" => {
            let mut m = t(l, Key::ProposeIncomplete).text();
            if retry_hint {
                m.push('\n');
                m.push_str(&t(l, Key::ProposeIncompleteRetry).text());
            }
            Some(m)
        }
        "quota_exceeded" => {
            let r = raw.cloned().unwrap_or_default();
            Some(format!(
                "{}\n{}",
                t(l, Key::ProposeQuota)
                    .with("quota", number(&r, "quota"))
                    .with("limit", number(&r, "limit"))
                    .with("used", number(&r, "used"))
                    .with("unit", number(&r, "unit"))
                    .text(),
                t(l, Key::ProposeQuotaRemedy).text()
            ))
        }
        "needs_recovery" => Some(t(l, Key::ProposeNeedsRecovery).text()),
        "declaration_misses_added" | "declared_not_held" => {
            Some(t(l, Key::ProposeDeclaration).text())
        }
        "base_unknown" => Some(t(l, Key::ProposeBaseUnknown).text()),
        "payload_too_large" => Some(t(l, Key::ServerTooLarge).text()),
        "too_large" => Some(t(l, Key::ProposeTooLargeForServer).text()),
        "proposal_not_open" => Some(t(l, Key::ProposeNotOpen).text()),
        "not_your_proposal" => Some(t(l, Key::ProposeNotYours).text()),
        "unknown_proposal" => Some(t(l, Key::ProposeUnknownProposal).text()),
        "no_new_content" => Some(t(l, Key::ProposeNoPatches).with("ref", "").text()),
        "intent_pending" => Some(
            t(l, Key::ServerBusy)
                .with("seconds", a.retry_after.unwrap_or(30))
                .text(),
        ),
        "accept_refused" => Some(
            t(l, Key::ProposeAcceptRefused)
                .with(
                    "reason",
                    raw.and_then(|r| r.get("detail"))
                        .and_then(|d| d.as_str())
                        .unwrap_or(""),
                )
                .text(),
        ),
        _ => None,
    };
    match message {
        Some(m) => {
            let mut f = Failure::new(REFUSED, m);
            if let Some(e) = &a.error {
                f = f.with_detail(format!(
                    "{} ({}): {} {}",
                    e.code, a.status, e.message, e.hint
                ));
            }
            f
        }
        None => http.failure(&HttpError::Server(Box::new(a.clone()))),
    }
}

pub fn run(ctx: &mut Ctx<'_>, args: &ProposeArgs) -> Result<(), Failure> {
    let url = parse_url(ctx, &args.url)?;
    if args.proposal.is_none() && args.title.is_none() && !args.dry_run {
        return Err(Failure::new(
            USAGE,
            t(ctx.lang, Key::ProposeTitleNeeded).text(),
        ));
    }
    let repo_dir = ctx.cwd.clone();
    if !Prikk::is_repository(&repo_dir) {
        return Err(Failure::new(
            USAGE,
            t(ctx.lang, Key::UsageNoRepository)
                .with("dir", repo_dir.display())
                .text(),
        ));
    }
    let prikk = Prikk::new(&repo_dir, ctx.lang, ctx.prikk_program.clone());
    prikk.check()?;
    let description = match &args.description_file {
        Some(p) => Some(std::fs::read_to_string(p).map_err(|e| local(ctx, p, &e))?),
        None => None,
    };
    let http = client(ctx, &url, &args.common, !args.dry_run)?;
    let git_ref = args.git_ref.as_str();
    // The branch must exist here before anything is asked of the server.
    prikk
        .run(
            "read your branch",
            &["log", "--format", "json", "--ref", git_ref, "--limit", "1"],
        )
        .map_err(|f| {
            Failure::new(
                crate::exit::USAGE,
                t(ctx.lang, Key::ProposeBranchGone)
                    .with("ref", git_ref)
                    .text(),
            )
            .with_detail(f.detail.unwrap_or(f.message))
        })?;

    ctx.say(
        t(ctx.lang, Key::ProposeStepAdvertise)
            .with("server", &url.host)
            .text(),
    );
    let advert: Advert = advertise(ctx, &http, &url)?;
    let ad_ref = advert.find(git_ref).filter(|r| r.published);
    let has_offer = ad_ref.is_some_and(|r| r.offers.iter().any(|(k, _, _)| k == "have"));
    if !has_offer {
        return Err(Failure::new(
            REFUSED,
            t(ctx.lang, Key::ProposeNoHave).with("ref", git_ref).text(),
        ));
    }
    let base = ad_ref.and_then(|r| r.tip_block.clone());

    let scratch = Scratch::new(ctx)?;
    let have_file = scratch.path().join("server.have");
    ctx.say(
        t(ctx.lang, Key::ProposeStepHave)
            .with("ref", git_ref)
            .text(),
    );
    let mut say_progress = |_: u64, _: Option<u64>| {};
    http.get_file(
        &url.api(&format!("transport/have?ref={}", query_value(git_ref))),
        &have_file,
        None,
        HAVE_MAX,
        &mut say_progress,
    )
    .map_err(|e| http.failure(&e))?;

    let Some(new_patches) = patches_above(&prikk, git_ref, base.as_deref())? else {
        return Err(Failure::new(
            REFUSED,
            t(ctx.lang, Key::ProposeBaseNotInHistory)
                .with("ref", git_ref)
                .text(),
        ));
    };
    if new_patches.is_empty() {
        ctx.say(
            t(ctx.lang, Key::ProposeNothingNew)
                .with("ref", git_ref)
                .text(),
        );
        return Ok(());
    }

    ctx.say(t(ctx.lang, Key::ProposeStepBuild).text());
    let artifact = scratch.path().join("work.artifact");
    let have_arg = have_file.to_string_lossy().into_owned();
    let art_arg = artifact.to_string_lossy().into_owned();
    prikk.run(
        "build your work",
        &[
            "sync", "build", git_ref, "--have", &have_arg, "--output", &art_arg,
        ],
    )?;
    let artifact_len = std::fs::metadata(&artifact)
        .map_err(|e| local(ctx, &artifact, &e))?
        .len();
    // The bound, when the server states one (the advertisement does not carry it yet: the server answers 413 before it reads a
    // byte of the body, so a too-large send costs one request).
    if let Some(limit) = advert
        .raw
        .get("limits")
        .and_then(|l| l.get("max_artifact_bytes"))
        .and_then(|v| v.as_u64())
        && artifact_len > limit
    {
        return Err(Failure::new(
            REFUSED,
            t(ctx.lang, Key::ProposeOverBound)
                .with("size", show_size(artifact_len))
                .with("limit", show_size(limit))
                .text(),
        ));
    }
    let base_text = base.clone().unwrap_or_else(|| "(nothing)".to_owned());
    ctx.say(
        t(ctx.lang, Key::ProposeStepDeclare)
            .with("count", new_patches.len())
            .with("base", short(&base_text))
            .text(),
    );

    if args.dry_run {
        let line = t(ctx.lang, Key::ProposeDryRun)
            .with("count", new_patches.len())
            .with("size", show_size(artifact_len))
            .with("base", short(&base_text))
            .with("url", url.display())
            .text();
        ctx.say(line);
        let shown: Vec<String> = new_patches.iter().take(20).map(|p| short(p)).collect();
        let more = if new_patches.len() > 20 { ", …" } else { "" };
        ctx.say(
            t(ctx.lang, Key::ProposeDryRunDetails)
                .with("patches", format!("{}{more}", shown.join(", ")))
                .text(),
        );
        return Ok(());
    }

    // An unfinished send is only repeated on request: the local record remembers it.
    let state_dir = state::state_dir(ctx.env);
    let mut record = state::load(&state_dir, &url.display(), ctx.lang)?;
    let work = fnv(&[
        &base_text,
        &new_patches.join(","),
        git_ref,
        args.proposal.as_deref().unwrap_or(""),
    ]);
    if record.unfinished.as_deref() == Some(work.as_str()) && !args.retry {
        let mut m = t(ctx.lang, Key::ProposeIncomplete).text();
        m.push('\n');
        m.push_str(&t(ctx.lang, Key::ProposeIncompleteRetry).text());
        return Err(Failure::new(REFUSED, m));
    }

    let mut meta = serde_json::json!({
        "target_ref": git_ref,
        "base": base,
        "declared_patches": new_patches,
        "client": format!("planeter-client/{}", env!("CARGO_PKG_VERSION")),
    });
    if let Some(p) = &args.proposal {
        meta["proposal"] = serde_json::json!(p);
    } else {
        meta["title"] = serde_json::json!(args.title);
        if let Some(d) = &description {
            meta["description"] = serde_json::json!(d);
        }
    }
    let meta_bytes = serde_json::to_vec(&meta).map_err(|e| {
        Failure::new(LOCAL, t(ctx.lang, Key::NetOther).text()).with_detail(e.to_string())
    })?;
    let mut prefix = Vec::with_capacity(MAGIC.len() + 8 + meta_bytes.len());
    prefix.extend_from_slice(MAGIC);
    prefix.extend_from_slice(&(meta_bytes.len() as u64).to_be_bytes());
    prefix.extend_from_slice(&meta_bytes);

    ctx.say(
        t(ctx.lang, Key::ProposeStepSend)
            .with("size", show_size(artifact_len + prefix.len() as u64))
            .text(),
    );
    let mut lines: Vec<(u64, u64)> = Vec::new();
    let posted = http.post_framed(
        &url.api("transport/propose"),
        &prefix,
        &artifact,
        &mut |s, t| lines.push((s, t)),
    );
    for (s, total) in lines {
        progress_line(ctx, Key::ProgressSent, s, Some(total));
    }
    match posted {
        Ok(Posted { status, body }) => {
            if record.unfinished.is_some() {
                record.unfinished = None;
                state::save(&state_dir, &record, ctx.lang)?;
            }
            let v: serde_json::Value = serde_json::from_slice(&body).map_err(|e| {
                Failure::new(NETWORK, t(ctx.lang, Key::NetBadAnswer).text())
                    .with_detail(e.to_string())
            })?;
            let id = v
                .get("proposal_id")
                .and_then(|x| x.as_str())
                .unwrap_or("?")
                .to_owned();
            let revision = v
                .get("revision")
                .and_then(|r| r.get("seq"))
                .map(|s| s.to_string())
                .unwrap_or_else(|| "?".to_owned());
            let page = url.page(&format!("proposals/{id}"));
            let already = v
                .get("already_recorded")
                .and_then(|b| b.as_bool())
                .unwrap_or(false);
            let created = v
                .get("created_proposal")
                .and_then(|b| b.as_bool())
                .unwrap_or(status == 201);
            let key = if already {
                Key::ProposeAlready
            } else if created {
                Key::ProposeDoneNew
            } else {
                Key::ProposeDoneRevision
            };
            let line = t(ctx.lang, key)
                .with("id", &id)
                .with("revision", &revision)
                .with("url", page)
                .text();
            ctx.say(line);
            if ctx.verbose
                && let Some(m) = v.get("message").and_then(|m| m.as_str())
            {
                ctx.say(format!("(server) {m}"));
            }
            Ok(())
        }
        Err(HttpError::Server(a)) => {
            if a.error.as_ref().is_some_and(|e| e.code == "incomplete") {
                record.unfinished = Some(work);
                state::save(&state_dir, &record, ctx.lang)?;
            }
            Err(refusal(ctx, &http, &a, true))
        }
        Err(e) => Err(http.failure(&e)),
    }
}

/// The first twelve characters of an id, for a line a person reads.
fn short(id: &str) -> String {
    id.chars().take(12).collect()
}
