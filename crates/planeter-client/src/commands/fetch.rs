//! `planeter-client fetch`: ask the server whether its state changed, download it if so, check it with the user's own prikk, make
//! sure the server has not shown two histories under one name (freshness, RFC 004 D-8), and land it as **received copies**. The
//! person's own branches are never touched; what to do with what came is said in plain words.

use crate::Ctx;
use crate::catalog::{Key, size as show_size, t};
use crate::cli::FetchArgs;
use crate::commands::{Advert, Scratch, advertise, client, parse_url, progress_line};
use crate::exit::{Failure, PRIKK, REFUSED};
use crate::http::{HttpError, Saved};
use crate::prikk::Prikk;
use crate::state;

const ARCHIVE_MAX: u64 = 64 << 30;

/// Is `tip` in the history of `git_ref` of the repository `prikk` is aimed at?
fn history_contains(prikk: &Prikk, git_ref: &str, tip: &str) -> Result<bool, Failure> {
    let mut limit = 512usize;
    loop {
        let Ok(v) = prikk.json(
            "read a received branch",
            &[
                "log",
                "--format",
                "json",
                "--ref",
                git_ref,
                "--limit",
                &limit.to_string(),
            ],
        ) else {
            return Ok(false);
        };
        let blocks = v
            .get("blocks")
            .and_then(|b| b.as_array())
            .cloned()
            .unwrap_or_default();
        if blocks
            .iter()
            .any(|b| b.get("block_id").and_then(|x| x.as_str()) == Some(tip))
        {
            return Ok(true);
        }
        if blocks.len() < limit {
            return Ok(false);
        }
        limit = limit.saturating_mul(4);
    }
}

pub fn run(ctx: &mut Ctx<'_>, args: &FetchArgs) -> Result<(), Failure> {
    let dir = match &args.dir {
        Some(d) => ctx.cwd.join(d),
        None => ctx.cwd.clone(),
    };
    let state_dir = state::state_dir(ctx.env);
    let not_a_clone = || {
        Failure::new(
            crate::exit::USAGE,
            t(ctx.lang, Key::FetchNotARepository)
                .with("dir", dir.display())
                .text(),
        )
    };
    if !Prikk::is_repository(&dir) {
        return Err(not_a_clone());
    }
    let origin = state::origin_of(&state_dir, &dir).ok_or_else(not_a_clone)?;
    let url = parse_url(ctx, &origin)?;
    let mine = Prikk::new(&dir, ctx.lang, ctx.prikk_program.clone());
    mine.check()?;
    let http = client(ctx, &url, &args.common, false)?;
    let mut rec = state::load(&state_dir, &url.display(), ctx.lang)?;

    ctx.say(
        t(ctx.lang, Key::FetchStepAsk)
            .with("server", &url.host)
            .text(),
    );
    let advert: Advert = advertise(ctx, &http, &url)?;
    ctx.say(t(ctx.lang, Key::FetchNoSequence).text());

    let scratch = Scratch::new(ctx)?;
    let archive = scratch.path().join("update.archive");
    let mut lines: Vec<(u64, Option<u64>)> = Vec::new();
    let saved = http
        .get_file(
            &url.api("transport/fetch?kind=archive"),
            &archive,
            rec.etag.as_deref(),
            ARCHIVE_MAX,
            &mut |d, total| lines.push((d, total)),
        )
        .map_err(|e: HttpError| http.failure(&e))?;
    let etag = match saved {
        Saved::NotModified => {
            ctx.say(t(ctx.lang, Key::FetchUnchanged).text());
            return Ok(());
        }
        Saved::Saved { etag, bytes } => {
            ctx.say(
                t(ctx.lang, Key::FetchStepDownload)
                    .with("size", show_size(bytes))
                    .text(),
            );
            etag
        }
    };
    for (d, total) in lines {
        progress_line(ctx, Key::ProgressDownloaded, d, total);
    }

    // Check it with prikk, offline, before anything of it touches the person's repository.
    let archive_arg = archive.to_string_lossy().into_owned();
    let report = mine.json(
        "check the archive",
        &["archive", "verify", &archive_arg, "--format", "json"],
    )?;
    if report.get("ok").and_then(|b| b.as_bool()) != Some(true) {
        return Err(Failure::new(
            PRIKK,
            t(ctx.lang, Key::CloneVerifyFailed)
                .with("dir", dir.display())
                .text(),
        )
        .with_detail(report.to_string()));
    }

    // Freshness: every branch this program was shown before must still be part of what the server shows now. The archive is
    // landed in a throwaway repository first; nothing of a split view reaches the person's own repository.
    let known: Vec<(&String, &state::RefRecord)> = rec
        .refs
        .iter()
        .filter(|(_, r)| r.tip_block.is_some())
        .collect();
    if !known.is_empty() {
        let trial_dir = scratch.path().join("trial");
        std::fs::create_dir_all(&trial_dir)
            .map_err(|e| crate::commands::local(ctx, &trial_dir, &e))?;
        let trial = Prikk::new(&trial_dir, ctx.lang, ctx.prikk_program.clone());
        trial.run("make a trial repository", &["init", "."])?;
        trial.run(
            "land the archive in a trial repository",
            &["archive", "import", &archive_arg],
        )?;
        for (name, r) in known {
            let received = format!("remotes/{name}");
            let tip = r.tip_block.as_deref().unwrap_or("");
            if !history_contains(&trial, &received, tip)? {
                return Err(Failure::new(
                    REFUSED,
                    t(ctx.lang, Key::FetchSplitView).with("ref", name).text(),
                ));
            }
        }
    }

    ctx.say(t(ctx.lang, Key::FetchStepImport).text());
    let out = mine.run(
        "land the archive as received copies",
        &["archive", "import", &archive_arg],
    )?;
    // What prikk says about what came (its classification, when it has one) is passed on as it is.
    let said = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if ctx.verbose && !said.is_empty() {
        ctx.say(said);
    }
    land_state(&mut rec, &advert, etag);
    state::save(&state_dir, &rec, ctx.lang)?;
    let count = advert.refs.len();
    ctx.say(t(ctx.lang, Key::FetchDone).with("refs", count).text());
    ctx.say(t(ctx.lang, Key::FetchDoneNext).text());
    Ok(())
}

fn land_state(rec: &mut state::Record, advert: &Advert, etag: Option<String>) {
    rec.etag = etag;
    rec.refs.clear();
    for r in advert.refs.iter().filter(|r| r.published) {
        rec.refs.insert(
            r.name.clone(),
            state::RefRecord {
                tip_block: r.tip_block.clone(),
                ref_state_id: r.ref_state_id.clone(),
            },
        );
    }
}
