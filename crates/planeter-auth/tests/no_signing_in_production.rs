//! RFC 010 D-4 regression pin for a crate that parses untrusted input (`jsonwebtoken`): the production code of
//! `planeter-auth` never names `EncodingKey` and never calls `encode(`. Signing exists only in
//! `#[cfg(test)]` code, which is the standing argument that the server never uses a private key (D-50: now
//! defence in depth, no longer the only defence against RUSTSEC-2023-0071 and RUSTSEC-2026-0387).
//!
//! The scan reads every `src/**/*.rs`, drops comments and every `#[cfg(test)]` item (a `mod tests` is one), and
//! looks for the two names in what is left. Planted cases prove the scan sees a violation and does not see a
//! name in a comment or in test code.

use std::fs;
use std::path::{Path, PathBuf};

/// The source text with comments blanked and every `#[cfg(test)]` item removed. String literals are kept
/// (a name inside one still counts) but their contents are not parsed for braces or comment markers.
fn production_text(src: &str) -> String {
    let b = src.as_bytes();
    let mut code = String::new();
    let mut i = 0;
    while i < b.len() {
        let rest = &src[i..];
        if rest.starts_with("//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if rest.starts_with("/*") {
            let mut depth = 0;
            while i < b.len() {
                if src[i..].starts_with("/*") {
                    depth += 1;
                    i += 2;
                } else if src[i..].starts_with("*/") {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            code.push(' ');
        } else if b[i] == b'"'
            || (b[i] == b'r' && (rest.starts_with("r\"") || rest.starts_with("r#")))
        {
            let (hashes, start) = if b[i] == b'r' {
                let h = rest[1..].bytes().take_while(|&c| c == b'#').count();
                (h, i + 2 + h)
            } else {
                (0, i + 1)
            };
            let close = format!("\"{}", "#".repeat(hashes));
            let mut j = start;
            while j < b.len() {
                if hashes == 0 && b[j] == b'\\' {
                    j += 2;
                } else if src[j..].starts_with(&close) {
                    j += close.len();
                    break;
                } else {
                    j += 1;
                }
            }
            let j = j.min(b.len());
            code.push_str(&src[i..j]);
            i = j;
        } else if b[i] == b'\'' {
            // A char literal ('x', '\n', '{') but not a lifetime ('a).
            let lit = if rest.starts_with("'\\") {
                rest.find('\'')
                    .and_then(|_| rest[2..].find('\'').map(|e| e + 3))
            } else {
                rest.get(1..)
                    .and_then(|r| r.chars().next())
                    .filter(|_| rest[1..].chars().nth(1) == Some('\''))
                    .map(|c| 2 + c.len_utf8())
            };
            match lit {
                Some(n) => {
                    code.push_str(&src[i..i + n]);
                    i += n;
                }
                None => {
                    code.push('\'');
                    i += 1;
                }
            }
        } else {
            let ch = rest.chars().next().unwrap();
            code.push(ch);
            i += ch.len_utf8();
        }
    }
    drop_cfg_test_items(&code)
}

/// Remove each `#[cfg(test)]` attribute together with the item it decorates: up to the matching `}` of the
/// first `{`, or to the first `;` when the item has no body.
fn drop_cfg_test_items(code: &str) -> String {
    let mut out = String::new();
    let mut rest = code;
    while let Some(at) = rest.find("#[cfg(test)]") {
        out.push_str(&rest[..at]);
        let after = &rest[at + "#[cfg(test)]".len()..];
        let (mut depth, mut end) = (0usize, after.len());
        let mut in_str = false;
        for (k, c) in after.char_indices() {
            match c {
                '"' => in_str = !in_str,
                '{' if !in_str => depth += 1,
                '}' if !in_str => {
                    depth -= 1;
                    if depth == 0 {
                        end = k + 1;
                        break;
                    }
                }
                ';' if !in_str && depth == 0 => {
                    end = k + 1;
                    break;
                }
                _ => {}
            }
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// The violations in one file's text: `EncodingKey` anywhere in production code, and the signing function
/// `encode` (named in a `jsonwebtoken` import or called as `encode(`).
fn violations(src: &str) -> Vec<&'static str> {
    let text = production_text(src);
    let mut found = Vec::new();
    if has_word(&text, "EncodingKey") {
        found.push("EncodingKey");
    }
    let calls = text.match_indices("encode(").any(|(k, _)| {
        let before = text[..k].chars().next_back();
        !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
            || text[..k].ends_with("jsonwebtoken::")
    });
    if calls || (text.contains("jsonwebtoken") && has_word(&text, "encode")) {
        found.push("encode(");
    }
    found
}

fn has_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(k, _)| {
        let before = text[..k].chars().next_back();
        let after = text[k + word.len()..].chars().next();
        !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
            && !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn production_code_of_planeter_auth_never_names_a_signing_key_or_calls_encode() {
    let mut files = Vec::new();
    sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(
        files.len() >= 8,
        "the scan found only {} source files; it is looking in the wrong place",
        files.len()
    );
    let mut bad = Vec::new();
    for f in &files {
        let v = violations(&fs::read_to_string(f).unwrap());
        if !v.is_empty() {
            bad.push(format!("{}: {v:?}", f.display()));
        }
    }
    assert!(
        bad.is_empty(),
        "signing in production code of planeter-auth (D-50, RFC 010 D-4): {bad:?}"
    );
    // The scan has seen the file whose test module does sign.
    let oidc =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/oidc.rs")).unwrap();
    assert!(
        oidc.contains("EncodingKey"),
        "oidc.rs no longer signs in its tests: the planted check below is the only proof of the scan"
    );
}

#[test]
fn planted_signing_in_production_code_is_found() {
    assert_eq!(
        violations("fn f() { let k = jsonwebtoken::EncodingKey::from_secret(b\"x\"); }"),
        vec!["EncodingKey"]
    );
    assert_eq!(
        violations("use jsonwebtoken::{Header, encode};\nfn f() {}"),
        vec!["encode("]
    );
    assert!(violations("fn f() { let t = encode(&h, &c, &k); }").contains(&"encode("));
    assert!(
        violations("fn f() { let t = jsonwebtoken::encode(&h, &c, &k); }").contains(&"encode(")
    );
    assert!(
        violations("fn f() { let s = \"EncodingKey\"; }").contains(&"EncodingKey"),
        "a string that names it counts"
    );
    // After a `#[cfg(test)]` item that ends, production code is scanned again.
    assert!(
        violations("#[cfg(test)]\nmod tests { fn t() {} }\nfn f() { let k: EncodingKey; }")
            .contains(&"EncodingKey")
    );
}

#[test]
fn planted_names_in_comments_and_test_code_are_not_violations() {
    assert!(violations("// EncodingKey and encode( are only named here\nfn f() {}").is_empty());
    assert!(
        violations("/// uses `EncodingKey::from_secret` in the docs\n/* encode( */\nfn f() {}")
            .is_empty()
    );
    assert!(violations("fn f() {}\n#[cfg(test)]\nmod tests {\n    use jsonwebtoken::{EncodingKey, encode};\n    fn t() { let s = \"}\"; let c = '}'; encode(&h, &c, &EncodingKey::from_secret(b\"x\")); }\n}\n").is_empty());
    assert!(
        violations("#[cfg(test)]\nfn only_in_tests() { let k: EncodingKey; }\nfn f() {}")
            .is_empty()
    );
    assert!(violations("#[cfg(test)]\nuse jsonwebtoken::EncodingKey;\nfn f() {}").is_empty());
    assert!(
        violations(
            "fn f() { let v = base64::encode_string(b\"x\"); let u = percent_encode(b\"y\"); }"
        )
        .is_empty()
    );
}
