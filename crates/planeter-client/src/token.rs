//! The bearer token: from `PLANETER_TOKEN` or `--token-file FILE`, never from the command line, and never printed (RFC 004 part 9, T1).
//!
//! `Token` has no `Display` and a redacted `Debug`; the only way to the text is [`Token::expose`], which the HTTP layer calls
//! once per request to build the header. A file is refused if other users can read it (Unix). **Windows:** the permission bits
//! are not a thing there, so the check is skipped; the file's access control list is the user profile's, private by default, and the
//! documentation (`docs/client.md`) says to keep the token file inside it.

use std::path::Path;

use crate::catalog::{Key, Lang, t};
use crate::exit::{Failure, LOCAL};

/// A token. It cannot be printed by accident.
#[derive(Clone)]
pub struct Token(String);

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(…)")
    }
}

impl Token {
    /// A token from its text: trimmed, and only visible ASCII (a header value must not carry anything else).
    pub fn new(text: &str) -> Option<Token> {
        let text = text.trim();
        (!text.is_empty() && text.bytes().all(|b| (0x21..=0x7e).contains(&b)))
            .then(|| Token(text.to_owned()))
    }

    /// The text, for the `Authorization` header only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

/// The token to use, if any: the file when given, else the environment.
pub fn load(
    file: Option<&Path>,
    env: impl Fn(&str) -> Option<String>,
    lang: Lang,
) -> Result<Option<Token>, Failure> {
    if let Some(path) = file {
        let shown = path.display().to_string();
        let meta = std::fs::metadata(path).map_err(|e| {
            Failure::new(
                LOCAL,
                t(lang, Key::TokenFileUnreadable)
                    .with("path", &shown)
                    .text(),
            )
            .with_detail(e.to_string())
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o077 != 0 {
                return Err(Failure::new(
                    LOCAL,
                    t(lang, Key::TokenFileTooOpen).with("path", &shown).text(),
                ));
            }
        }
        #[cfg(not(unix))]
        let _ = meta;
        let text = std::fs::read_to_string(path).map_err(|e| {
            Failure::new(
                LOCAL,
                t(lang, Key::TokenFileUnreadable)
                    .with("path", &shown)
                    .text(),
            )
            .with_detail(e.to_string())
        })?;
        return Token::new(&text).map(Some).ok_or_else(|| {
            Failure::new(
                LOCAL,
                t(lang, Key::TokenFileEmpty).with("path", &shown).text(),
            )
        });
    }
    Ok(env("PLANETER_TOKEN").and_then(|v| Token::new(&v)))
}
