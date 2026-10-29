//! `planeter-client clone`: download the repository, check it with the user's own prikk, show the maintainer keys it carries with
//! their whole public key in groups, and trust **only the keys the person types** (or gives with `--trust-key`). There is no
//! default "yes": an empty answer skips the key. `--no-adopt` keeps everything as received copies and trusts nothing.

use std::path::{Path, PathBuf};

use crate::Ctx;
use crate::catalog::{Key, t};
use crate::cli::CloneArgs;
use crate::commands::{
    Scratch, advertise, client, group, local, normalise_key, parse_url, progress_line,
};
use crate::exit::{Failure, LOCAL, PRIKK, REFUSED, USAGE};
use crate::http::{HttpError, Saved};
use crate::prikk::Prikk;
use crate::state;

/// The most a repository archive may be (a guard on what is written to disk; the server's own bound is the real one).
const ARCHIVE_MAX: u64 = 64 << 30;

struct Key_ {
    key_id: String,
    public_key: String,
}

/// Which of the carried keys the person trusts. Interactive: type each key; empty skips it. A wrong key is told so and asked
/// again (twice), then skipped.
fn ask_keys(ctx: &mut Ctx<'_>, keys: &[Key_]) -> Result<Vec<usize>, Failure> {
    let mut trusted = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        let n = i + 1;
        let shown = t(ctx.lang, Key::CloneKeyShow)
            .with("number", n)
            .with("count", keys.len())
            .with("key", group(&k.public_key))
            .text();
        ctx.say(shown);
        let mut attempts = 0;
        loop {
            let ask = t(ctx.lang, Key::CloneKeyAsk).with("number", n).text();
            let _ = writeln!(ctx.out, "{ask}");
            let _ = ctx.out.flush();
            let mut line = String::new();
            let read = ctx.stdin.read_line(&mut line).unwrap_or(0);
            let typed = normalise_key(&line);
            if read == 0 || typed.is_empty() {
                ctx.say(t(ctx.lang, Key::CloneKeySkipped).with("number", n).text());
                break;
            }
            if typed == k.public_key.to_ascii_lowercase() {
                trusted.push(i);
                ctx.say(t(ctx.lang, Key::CloneKeyTrusted).with("number", n).text());
                break;
            }
            attempts += 1;
            ctx.say(t(ctx.lang, Key::CloneKeyWrong).with("number", n).text());
            if attempts >= 2 {
                ctx.say(t(ctx.lang, Key::CloneKeySkipped).with("number", n).text());
                break;
            }
        }
    }
    Ok(trusted)
}

pub fn run(ctx: &mut Ctx<'_>, args: &CloneArgs) -> Result<(), Failure> {
    let url = parse_url(ctx, &args.url)?;
    let dir: PathBuf = match &args.dir {
        Some(d) => ctx.cwd.join(d),
        None => ctx.cwd.join(&url.name),
    };
    if dir.exists()
        && std::fs::read_dir(&dir)
            .map(|mut r| r.next().is_some())
            .unwrap_or(true)
    {
        return Err(Failure::new(
            LOCAL,
            t(ctx.lang, Key::CloneDirExists)
                .with("dir", dir.display())
                .text(),
        ));
    }
    // A person who cannot type, and gave no key, gets an answer before anything is downloaded.
    if !args.no_adopt && args.trust_keys.is_empty() && !ctx.interactive {
        return Err(Failure::new(
            USAGE,
            t(ctx.lang, Key::CloneNoTerminal).text(),
        ));
    }
    let scratch = Scratch::new(ctx)?;
    let probe = Prikk::new(scratch.path(), ctx.lang, ctx.prikk_program.clone());
    probe.check()?;
    let http = client(ctx, &url, &args.common, false)?;

    let advert = advertise(ctx, &http, &url)?;
    ctx.say(
        t(ctx.lang, Key::CloneStepDownload)
            .with("server", &url.host)
            .text(),
    );
    let archive = scratch.path().join("repository.archive");
    let mut lines: Vec<(u64, Option<u64>)> = Vec::new();
    let saved = http
        .get_file(
            &url.api("transport/fetch?kind=archive"),
            &archive,
            None,
            ARCHIVE_MAX,
            &mut |d, total| lines.push((d, total)),
        )
        .map_err(|e: HttpError| http.failure(&e))?;
    for (d, total) in lines {
        progress_line(ctx, Key::ProgressDownloaded, d, total);
    }
    let etag = match saved {
        Saved::Saved { etag, .. } => etag,
        Saved::NotModified => None,
    };

    ctx.say(t(ctx.lang, Key::CloneStepVerify).text());
    let archive_arg = archive.to_string_lossy().into_owned();
    let report = probe
        .json(
            "check the archive",
            &["archive", "verify", &archive_arg, "--format", "json"],
        )
        .map_err(|f| {
            Failure::new(
                PRIKK,
                t(ctx.lang, Key::CloneArchiveBad)
                    .with("dir", dir.display())
                    .text(),
            )
            .with_detail(f.detail.unwrap_or(f.message))
        })?;
    if report.get("ok").and_then(|b| b.as_bool()) != Some(true) {
        return Err(Failure::new(
            PRIKK,
            t(ctx.lang, Key::CloneVerifyFailed)
                .with("dir", dir.display())
                .text(),
        )
        .with_detail(report.to_string()));
    }
    let keys: Vec<Key_> = report
        .get("maintainer_keys")
        .and_then(|k| k.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|k| {
                    Some(Key_ {
                        key_id: k.get("key_id")?.as_str()?.to_owned(),
                        public_key: k.get("public_key")?.as_str()?.to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Which keys are trusted: only the ones typed (or named with --trust-key); never a default.
    let mut trusted: Vec<usize> = Vec::new();
    if !args.no_adopt {
        if !args.trust_keys.is_empty() {
            for given in &args.trust_keys {
                let want = normalise_key(given);
                match keys
                    .iter()
                    .position(|k| k.public_key.to_ascii_lowercase() == want)
                {
                    Some(i) => {
                        if !trusted.contains(&i) {
                            trusted.push(i);
                        }
                    }
                    None => {
                        return Err(Failure::new(
                            REFUSED,
                            t(ctx.lang, Key::CloneKeyNotCarried)
                                .with("key", group(&want))
                                .text(),
                        ));
                    }
                }
            }
        } else if keys.is_empty() {
            ctx.say(t(ctx.lang, Key::CloneKeysNone).text());
        } else {
            ctx.say(
                t(ctx.lang, Key::CloneKeysHeader)
                    .with("count", keys.len())
                    .text(),
            );
            trusted = ask_keys(ctx, &keys)?;
        }
    }

    // Only now is anything put where the person asked.
    ctx.say(
        t(ctx.lang, Key::CloneStepImport)
            .with("dir", dir.display())
            .text(),
    );
    let created_here = !dir.exists();
    let outcome = land(ctx, &dir, &archive, &keys, &trusted, args.no_adopt);
    if let Err(e) = outcome {
        if created_here {
            let _ = std::fs::remove_dir_all(&dir);
        }
        return Err(e);
    }
    // What this program remembers: where the clone came from, and what the server showed.
    let state_dir = state::state_dir(ctx.env);
    state::remember_origin(&state_dir, &dir, &url.display(), ctx.lang)?;
    let mut rec = state::load(&state_dir, &url.display(), ctx.lang)?;
    rec.etag = etag;
    rec.refs.clear();
    state::record_refs(&mut rec, &advert);
    state::save(&state_dir, &rec, ctx.lang)?;

    if args.no_adopt || trusted.is_empty() {
        ctx.say(t(ctx.lang, Key::CloneNoAdopt).text());
    }
    let done = t(ctx.lang, Key::CloneDone)
        .with("name", &url.name)
        .with("dir", dir.display())
        .with("trusted", trusted.len())
        .with("count", keys.len())
        .text();
    ctx.say(done);
    ctx.say(
        t(ctx.lang, Key::CloneDoneNext)
            .with("dir", dir.display())
            .text(),
    );
    Ok(())
}

/// Make the repository, trust the chosen keys, land the archive, and let prikk check the result. With no key trusted (`--no-adopt`,
/// or every key skipped) nothing can be verified or trusted: the archive lands as received copies, and prikk is not asked to
/// verify a repository that trusts nobody.
fn land(
    ctx: &Ctx<'_>,
    dir: &Path,
    archive: &Path,
    keys: &[Key_],
    trusted: &[usize],
    no_adopt: bool,
) -> Result<(), Failure> {
    std::fs::create_dir_all(dir).map_err(|e| local(ctx, dir, &e))?;
    let prikk = Prikk::new(dir, ctx.lang, ctx.prikk_program.clone());
    prikk.run("make the repository", &["init", "."])?;
    for &i in trusted {
        prikk.run(
            "trust the key you typed",
            &[
                "trust",
                "maintainer",
                "add",
                "--key-id",
                &keys[i].key_id,
                "--public-key",
                &keys[i].public_key,
            ],
        )?;
    }
    let archive_arg = archive.to_string_lossy().into_owned();
    if no_adopt || trusted.is_empty() {
        prikk.run(
            "land the archive as received copies",
            &["archive", "import", &archive_arg],
        )?;
    } else {
        prikk.run(
            "land the archive",
            &["archive", "import", &archive_arg, "--adopt"],
        )?;
        prikk.run("check the repository", &["verify"])?;
    }
    Ok(())
}
