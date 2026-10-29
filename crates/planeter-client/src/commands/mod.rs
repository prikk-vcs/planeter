//! The three commands, and what they share.

pub mod clone;
pub mod fetch;
pub mod propose;

use std::path::{Path, PathBuf};

use crate::Ctx;
use crate::catalog::{Key, t};
use crate::cli::Common;
use crate::exit::{Failure, LOCAL, USAGE};
use crate::http::{Http, plain_http_failure};
use crate::token;
use crate::url::{RepoUrl, UrlError};

/// Parse the address a person gave, as a sentence when it is wrong.
pub fn parse_url(ctx: &Ctx<'_>, text: &str) -> Result<RepoUrl, Failure> {
    RepoUrl::parse(text).map_err(|e| match e {
        UrlError::PlainHttp => plain_http_failure(ctx.lang, text),
        UrlError::Bad => Failure::new(
            USAGE,
            t(ctx.lang, Key::UsageBadUrl).with("url", text).text(),
        ),
    })
}

/// A client for `url`, with the token (when one is given) and the trust anchors the person chose.
pub fn client(
    ctx: &Ctx<'_>,
    url: &RepoUrl,
    common: &Common,
    need_token: bool,
) -> Result<Http, Failure> {
    let tok = token::load(common.token_file.as_deref(), ctx.env, ctx.lang)?;
    if need_token && tok.is_none() {
        return Err(Failure::new(USAGE, t(ctx.lang, Key::TokenRequired).text()));
    }
    Http::new(
        &url.host,
        common.cacert.as_deref(),
        tok,
        &ctx.timeouts,
        ctx.lang,
        ctx.verbose,
    )
}

/// A working directory of this run, removed when it goes out of scope (also on an error).
pub struct Scratch {
    path: PathBuf,
}

impl Scratch {
    pub fn new(ctx: &Ctx<'_>) -> Result<Scratch, Failure> {
        let base = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = base.join(format!("planeter-client-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).map_err(|e| {
            Failure::new(
                LOCAL,
                t(ctx.lang, Key::LocalFile)
                    .with("path", path.display())
                    .with("reason", e.to_string())
                    .text(),
            )
        })?;
        Ok(Scratch { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A local file problem as a failure.
pub fn local(ctx: &Ctx<'_>, path: &Path, e: &std::io::Error) -> Failure {
    Failure::new(
        LOCAL,
        t(ctx.lang, Key::LocalFile)
            .with("path", path.display())
            .with("reason", e.to_string())
            .text(),
    )
    .with_detail(e.to_string())
}

/// A progress line: whole lines only, never a redraw, so a pipe or a log file reads cleanly.
pub fn progress_line(ctx: &mut Ctx<'_>, key: Key, done: u64, total: Option<u64>) {
    let line = t(ctx.lang, key)
        .with("size", crate::catalog::size(done))
        .with(
            "total",
            total.map_or_else(|| "?".to_owned(), crate::catalog::size),
        )
        .text();
    ctx.say(line);
}

/// What the advertisement says about one ref (the fields a command reads).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdRef {
    pub name: String,
    pub published: bool,
    pub kind: String,
    pub tip_block: Option<String>,
    pub ref_state_id: Option<String>,
    pub offers: Vec<(String, Option<u64>, String)>,
}

/// The advertisement of a repository, reduced to what the commands use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advert {
    pub fingerprint: String,
    pub refs: Vec<AdRef>,
    pub archive_etag: Option<String>,
    pub archive_size: Option<u64>,
    pub raw: serde_json::Value,
}

impl Advert {
    pub fn parse(v: serde_json::Value) -> Option<Advert> {
        let text =
            |x: &serde_json::Value, k: &str| x.get(k).and_then(|s| s.as_str()).map(str::to_owned);
        let refs = v
            .get("refs")?
            .as_array()?
            .iter()
            .map(|r| AdRef {
                name: text(r, "name").unwrap_or_default(),
                published: r.get("state").and_then(|s| s.as_str()) == Some("published"),
                kind: text(r, "kind").unwrap_or_default(),
                tip_block: text(r, "tip_block"),
                ref_state_id: text(r, "ref_state_id"),
                offers: r
                    .get("fetch")
                    .and_then(|f| f.as_array())
                    .map(|a| {
                        a.iter()
                            .map(|o| {
                                (
                                    text(o, "kind").unwrap_or_default(),
                                    o.get("size").and_then(|s| s.as_u64()),
                                    text(o, "etag").unwrap_or_default(),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect();
        let archive = v.get("archive");
        Some(Advert {
            fingerprint: text(&v, "fingerprint")?,
            refs,
            archive_etag: archive.and_then(|a| text(a, "etag")),
            archive_size: archive.and_then(|a| a.get("size").and_then(|s| s.as_u64())),
            raw: v,
        })
    }

    pub fn find(&self, name: &str) -> Option<&AdRef> {
        self.refs.iter().find(|r| r.name == name)
    }
}

/// Fetch and parse the advertisement.
pub fn advertise(ctx: &Ctx<'_>, http: &Http, url: &RepoUrl) -> Result<Advert, Failure> {
    let v = http
        .get_json(&url.api("transport/advertise"))
        .map_err(|e| http.failure(&e))?;
    Advert::parse(v).ok_or_else(|| {
        Failure::new(crate::exit::NETWORK, t(ctx.lang, Key::NetBadAnswer).text())
            .with_detail("the advertisement lacks `fingerprint` or `refs`")
    })
}

/// Group a public key in blocks of eight characters, for reading aloud and comparing by eye.
pub fn group(key: &str) -> String {
    key.as_bytes()
        .chunks(8)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A key as typed, reduced to what is compared: lower-case hexadecimal, spaces, colons and dashes dropped.
pub fn normalise_key(typed: &str) -> String {
    typed
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, ':' | '-'))
        .collect::<String>()
        .to_ascii_lowercase()
}
