//! The text prikk prints for `sync accept` and `sync pending`, read into typed values (RFC 004 D-3, U-12).
//!
//! Neither verb has a JSON form (prikk 0.53.0), so these two readers are the only places that depend on the shape
//! of their text, and they are strict about the parts a record is derived from:
//!
//! * `sync accept` prints a summary (`patches: N`, `blobs: N`, `claims: N`, `tags: N`, `new objects: N`) and one
//!   line per claim, `claim <64 hex>: <outcome>`; every line that starts with `claim ` must have that shape, and
//!   their number must equal `claims: N`, otherwise the output is **unreadable** ([`PrikkError::Parse`]) and no
//!   revision may be derived from it. Lines the readers do not know are ignored (a later prikk may add some).
//! * `sync pending` prints `pending (accepted, unsealed) patches: N` and one indented 64-hex patch id per line;
//!   the count in the header must equal the number of ids, otherwise the output is unreadable.

use crate::error::{PrikkError, Result};

/// One claim line of `sync accept`: the claim id and the outcome as prikk said it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimLine {
    pub claim_id: String,
    /// Everything after `claim <id>: `, verbatim (for example `Sound (ed25519-4e47dc83e8e323ef)`).
    pub outcome: String,
    /// The author key ids named in the outcome (`ed25519-` and hexadecimal), in order of appearance.
    pub key_ids: Vec<String>,
}

/// What a completed `sync accept` reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptReport {
    pub patches: u64,
    pub blobs: u64,
    pub claims: u64,
    pub tags: u64,
    pub new_objects: u64,
    pub claim_lines: Vec<ClaimLine>,
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The key ids (`ed25519-<hex>`) named in `text`.
fn key_ids(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("ed25519-") {
        let tail = &rest[at..];
        let hex_len = tail["ed25519-".len()..]
            .bytes()
            .take_while(u8::is_ascii_hexdigit)
            .count();
        if hex_len > 0 {
            let id = &tail[..("ed25519-".len() + hex_len)];
            if !out.iter().any(|k| k == id) {
                out.push(id.to_owned());
            }
        }
        rest = &tail["ed25519-".len() + hex_len..];
    }
    out
}

fn unreadable(what: &str) -> PrikkError {
    PrikkError::Parse(format!("unreadable `sync` output: {what}"))
}

/// Read the stdout of a successful `sync accept`.
pub fn parse_accept_output(stdout: &str) -> Result<AcceptReport> {
    let mut first = true;
    let (mut patches, mut blobs, mut claims, mut tags, mut new_objects) =
        (None, None, None, None, None);
    let mut claim_lines = Vec::new();
    for line in stdout.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if first {
            first = false;
            if line != "accepted sync artifact" {
                return Err(unreadable("the first line is not `accepted sync artifact`"));
            }
            continue;
        }
        let number = |prefix: &str| -> Option<Result<u64>> {
            line.strip_prefix(prefix).map(|v| {
                v.trim()
                    .parse::<u64>()
                    .map_err(|_| unreadable("a count is not a number"))
            })
        };
        if let Some(n) = number("patches:") {
            patches = Some(n?);
        } else if let Some(n) = number("blobs:") {
            blobs = Some(n?);
        } else if let Some(n) = number("claims:") {
            claims = Some(n?);
        } else if let Some(n) = number("tags:") {
            tags = Some(n?);
        } else if let Some(n) = number("new objects:") {
            new_objects = Some(n?);
        } else if let Some(rest) = line.strip_prefix("claim ") {
            let (id, outcome) = rest
                .split_once(": ")
                .ok_or_else(|| unreadable("a claim line has no `: `"))?;
            if !is_hex64(id) {
                return Err(unreadable("a claim id is not 64 hexadecimal characters"));
            }
            claim_lines.push(ClaimLine {
                claim_id: id.to_owned(),
                outcome: outcome.trim().to_owned(),
                key_ids: key_ids(outcome),
            });
        }
    }
    if first {
        return Err(unreadable("empty output"));
    }
    let need =
        |v: Option<u64>, name: &str| v.ok_or_else(|| unreadable(&format!("no `{name}:` line")));
    let report = AcceptReport {
        patches: need(patches, "patches")?,
        blobs: need(blobs, "blobs")?,
        claims: need(claims, "claims")?,
        tags: need(tags, "tags")?,
        new_objects: need(new_objects, "new objects")?,
        claim_lines,
    };
    if report.claim_lines.len() as u64 != report.claims {
        return Err(unreadable(
            "the number of claim lines is not the `claims:` count",
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    if !report
        .claim_lines
        .iter()
        .all(|c| seen.insert(c.claim_id.as_str()))
    {
        return Err(unreadable("a claim id is listed twice"));
    }
    Ok(report)
}

/// Read the claim ids of a `--claims-out` file: one 64-hex id per line.
pub fn parse_claims_file(text: &str) -> Result<Vec<String>> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            if is_hex64(l) {
                Ok(l.to_owned())
            } else {
                Err(unreadable(
                    "the claims file holds a line that is not a claim id",
                ))
            }
        })
        .collect()
}

/// Read the stdout of `sync pending` into patch ids, in the order printed.
pub fn parse_pending_output(stdout: &str) -> Result<Vec<String>> {
    let mut lines = stdout.lines().map(str::trim).filter(|l| !l.is_empty());
    let header = lines
        .next()
        .ok_or_else(|| unreadable("empty pending output"))?;
    let count: u64 = header
        .strip_prefix("pending (accepted, unsealed) patches:")
        .ok_or_else(|| unreadable("the first line is not the pending header"))?
        .trim()
        .parse()
        .map_err(|_| unreadable("the pending count is not a number"))?;
    let ids: Vec<String> = lines
        .map(|l| {
            if is_hex64(l) {
                Ok(l.to_owned())
            } else {
                Err(unreadable("a pending line is not a patch id"))
            }
        })
        .collect::<Result<_>>()?;
    if ids.len() as u64 != count {
        return Err(unreadable(
            "the pending count is not the number of ids listed",
        ));
    }
    Ok(ids)
}
