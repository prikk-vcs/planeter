#![forbid(unsafe_code)]
//! The image gate's identity check (D-51, `image-identity-handoff-v1`): the release job loads the image it built, runs it, checks
//! it, and pushes an image built by a second `buildx build`. This reads what the **registry** holds after the push and decides
//! whether it is the image that was run:
//!
//! 1. [`platform_manifest`]: from the pushed digest's index, the digest of the manifest for `linux/<arch>` (the attestation
//!    manifests, whose platform is `unknown/unknown`, are not images and never match);
//! 2. [`config_digest`]: from that manifest, the digest of its config blob, which is what a local image id is;
//! 3. [`same`]: it must equal the id of the image that was run, and a mismatch names both.
//!
//! Every function is strict: an input that is not what it says it is (malformed JSON, not an index, not a digest) is a
//! [`Failure::Usage`], never a pass; a registry that does not hold the image is a [`Failure::Mismatch`].

use std::fmt;

/// Why the check did not pass. `Usage` is the caller's or the registry's input being unreadable (exit 2); `Mismatch` is a
/// reading that succeeded and said no (exit 1).
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    Usage(String),
    Mismatch(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Usage(m) | Failure::Mismatch(m) => f.write_str(m),
        }
    }
}

/// A JSON value; numbers keep their text (nothing here computes with them).
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
}

const MAX_DEPTH: usize = 32;

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}

/// Parse one JSON document (nothing may follow it but white space).
pub fn parse(text: &str) -> Result<Json, String> {
    let mut p = Parser {
        bytes: text.as_bytes(),
        at: 0,
    };
    let v = p.value(0)?;
    p.space();
    if p.at != p.bytes.len() {
        return Err(format!(
            "unexpected text after the JSON value at byte {}",
            p.at
        ));
    }
    Ok(v)
}

impl Parser<'_> {
    fn space(&mut self) {
        while matches!(self.bytes.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn eat(&mut self, b: u8) -> Result<(), String> {
        if self.bytes.get(self.at) == Some(&b) {
            self.at += 1;
            Ok(())
        } else {
            Err(format!("expected {:?} at byte {}", b as char, self.at))
        }
    }

    fn word(&mut self, w: &str, v: Json) -> Result<Json, String> {
        if self.bytes[self.at..].starts_with(w.as_bytes()) {
            self.at += w.len();
            Ok(v)
        } else {
            Err(format!("unexpected text at byte {}", self.at))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > MAX_DEPTH {
            return Err("JSON nested too deeply".to_owned());
        }
        self.space();
        match self.bytes.get(self.at) {
            None => Err("the JSON ends too early".to_owned()),
            Some(b'{') => {
                self.at += 1;
                let mut members = Vec::new();
                self.space();
                if self.bytes.get(self.at) == Some(&b'}') {
                    self.at += 1;
                    return Ok(Json::Obj(members));
                }
                loop {
                    self.space();
                    let key = self.string()?;
                    self.space();
                    self.eat(b':')?;
                    let v = self.value(depth + 1)?;
                    members.push((key, v));
                    self.space();
                    match self.bytes.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b'}') => {
                            self.at += 1;
                            return Ok(Json::Obj(members));
                        }
                        _ => return Err(format!("expected ',' or '}}' at byte {}", self.at)),
                    }
                }
            }
            Some(b'[') => {
                self.at += 1;
                let mut items = Vec::new();
                self.space();
                if self.bytes.get(self.at) == Some(&b']') {
                    self.at += 1;
                    return Ok(Json::Arr(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.space();
                    match self.bytes.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b']') => {
                            self.at += 1;
                            return Ok(Json::Arr(items));
                        }
                        _ => return Err(format!("expected ',' or ']' at byte {}", self.at)),
                    }
                }
            }
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.word("true", Json::Bool(true)),
            Some(b'f') => self.word("false", Json::Bool(false)),
            Some(b'n') => self.word("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => {
                let start = self.at;
                while matches!(
                    self.bytes.get(self.at),
                    Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                ) {
                    self.at += 1;
                }
                let n = &self.bytes[start..self.at];
                let text = String::from_utf8_lossy(n).into_owned();
                if text.parse::<f64>().is_err() {
                    return Err(format!("not a number at byte {start}"));
                }
                Ok(Json::Num(text))
            }
            Some(_) => Err(format!("unexpected character at byte {}", self.at)),
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let end = self.at + 4;
        let digits = self
            .bytes
            .get(self.at..end)
            .and_then(|b| std::str::from_utf8(b).ok())
            .and_then(|s| u32::from_str_radix(s, 16).ok())
            .ok_or_else(|| format!("bad \\u escape at byte {}", self.at))?;
        self.at = end;
        Ok(digits)
    }

    fn string(&mut self) -> Result<String, String> {
        self.eat(b'"')?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&b) = self.bytes.get(self.at) else {
                return Err("a string is not closed".to_owned());
            };
            self.at += 1;
            match b {
                b'"' => break,
                b'\\' => {
                    let Some(&e) = self.bytes.get(self.at) else {
                        return Err("a string is not closed".to_owned());
                    };
                    self.at += 1;
                    let c = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi)
                                && self.bytes[self.at..].starts_with(b"\\u")
                            {
                                self.at += 2;
                                let lo = self.hex4()?;
                                0x10000 + ((hi - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF)
                            } else {
                                hi
                            };
                            char::from_u32(code).unwrap_or('\u{FFFD}')
                        }
                        _ => return Err(format!("bad escape at byte {}", self.at - 1)),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                }
                0..=0x1f => {
                    return Err(format!(
                        "a control character in a string at byte {}",
                        self.at - 1
                    ));
                }
                _ => out.push(b),
            }
        }
        String::from_utf8(out).map_err(|_| "a string is not UTF-8".to_owned())
    }
}

/// `sha256:` and 64 lower-case hex digits: what an image id, a manifest digest and a config digest look like.
pub fn is_digest(text: &str) -> bool {
    text.strip_prefix("sha256:").is_some_and(|h| {
        h.len() == 64
            && h.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

fn digest_field<'a>(v: &'a Json, what: &str) -> Result<&'a str, Failure> {
    let d = v
        .as_str()
        .ok_or_else(|| Failure::Usage(format!("{what} is not a string")))?;
    if is_digest(d) {
        Ok(d)
    } else {
        Err(Failure::Usage(format!(
            "{what} is not a sha256 digest: {d:?}"
        )))
    }
}

/// From the index a pushed digest names, the digest of the manifest for `linux/<arch>`. An entry counts only if it says
/// `os: linux` and `architecture: <arch>`: the attestation manifests (`unknown/unknown`) are not images. No match, or two
/// (ambiguous), is a mismatch; text that is not an image index is a usage failure.
pub fn platform_manifest(index_json: &str, arch: &str) -> Result<String, Failure> {
    let index =
        parse(index_json).map_err(|e| Failure::Usage(format!("the index is not JSON: {e}")))?;
    let Some(Json::Arr(entries)) = index.get("manifests") else {
        return Err(Failure::Usage(
            "the document has no `manifests` list: it is not an image index (a single manifest was pushed?)".to_owned(),
        ));
    };
    let mut found: Vec<&str> = Vec::new();
    for entry in entries {
        let platform = entry.get("platform");
        let says = |key: &str| platform.and_then(|p| p.get(key)).and_then(Json::as_str);
        if says("os") == Some("linux") && says("architecture") == Some(arch) {
            let d = entry
                .get("digest")
                .ok_or_else(|| Failure::Usage("a manifest entry has no digest".to_owned()))?;
            found.push(digest_field(d, "a manifest entry's digest")?);
        }
    }
    match found.as_slice() {
        [one] => Ok((*one).to_owned()),
        [] => Err(Failure::Mismatch(format!(
            "the pushed index holds no image for linux/{arch} ({} entries, none of them that platform)",
            entries.len()
        ))),
        many => Err(Failure::Mismatch(format!(
            "the pushed index holds {} images for linux/{arch}; which one was run is not decidable",
            many.len()
        ))),
    }
}

/// From an image manifest, the digest of its config blob (which is the id a local image has).
pub fn config_digest(manifest_json: &str) -> Result<String, Failure> {
    let manifest = parse(manifest_json)
        .map_err(|e| Failure::Usage(format!("the manifest is not JSON: {e}")))?;
    let config = manifest.get("config").ok_or_else(|| {
        Failure::Usage("the document has no `config`: it is not an image manifest".to_owned())
    })?;
    let d = config
        .get("digest")
        .ok_or_else(|| Failure::Usage("the manifest's config has no digest".to_owned()))?;
    digest_field(d, "the manifest's config digest").map(str::to_owned)
}

/// The image that was run and the image that was pushed must be one: equal digests, each of them a digest.
pub fn same(run: &str, pushed: &str) -> Result<(), Failure> {
    let run = run.trim();
    let pushed = pushed.trim();
    if !is_digest(run) {
        return Err(Failure::Usage(format!(
            "the id of the image that was run is not a sha256 digest: {run:?}"
        )));
    }
    if !is_digest(pushed) {
        return Err(Failure::Usage(format!(
            "the config digest of the pushed image is not a sha256 digest: {pushed:?}"
        )));
    }
    if run == pushed {
        Ok(())
    } else {
        Err(Failure::Mismatch(format!(
            "the pushed image is not the image that was run: it ran as {run} and was pushed as {pushed}"
        )))
    }
}
