//! The address of a repository on a planeter server: `https://host[:port]/owner/name`. https only; no user name, no query, no
//! fragment; exactly the two path segments.

/// Why an address was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlError {
    /// It starts with `http://` (never followed, never allowed).
    PlainHttp,
    /// It is not the address of a repository.
    Bad,
}

/// A parsed repository address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoUrl {
    /// `https://host` or `https://host:port`.
    pub origin: String,
    pub host: String,
    pub owner: String,
    pub name: String,
}

fn segment_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn host_ok(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 253
        && h.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
}

impl RepoUrl {
    pub fn parse(text: &str) -> Result<RepoUrl, UrlError> {
        let text = text.trim();
        let lower = text.to_ascii_lowercase();
        if lower.starts_with("http://") {
            return Err(UrlError::PlainHttp);
        }
        let Some(rest) = lower.strip_prefix("https://") else {
            return Err(UrlError::Bad);
        };
        // Keep the case of the path (owner and repository names), lower-case only the host.
        let after_scheme = &text["https://".len()..];
        let _ = rest;
        if after_scheme.contains(['?', '#', '@', '\\']) {
            return Err(UrlError::Bad);
        }
        let (authority, path) = after_scheme.split_once('/').ok_or(UrlError::Bad)?;
        if !host_ok(authority) {
            return Err(UrlError::Bad);
        }
        let segments: Vec<&str> = path.trim_end_matches('/').split('/').collect();
        let [owner, name] = segments[..] else {
            return Err(UrlError::Bad);
        };
        if !segment_ok(owner) || !segment_ok(name) {
            return Err(UrlError::Bad);
        }
        let authority = authority.to_ascii_lowercase();
        let host = authority
            .rsplit_once(':')
            .filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit()) && !port.is_empty())
            .map_or(authority.as_str(), |(h, _)| h)
            .to_owned();
        Ok(RepoUrl {
            origin: format!("https://{authority}"),
            host,
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }

    /// `https://host/api/v1/repos/owner/name/<tail>`.
    pub fn api(&self, tail: &str) -> String {
        format!(
            "{}/api/v1/repos/{}/{}/{}",
            self.origin, self.owner, self.name, tail
        )
    }

    /// `https://host/owner/name/<tail>`: a page a person opens.
    pub fn page(&self, tail: &str) -> String {
        format!("{}/{}/{}/{}", self.origin, self.owner, self.name, tail)
    }

    /// The address as it was given, normalised (for state keys and messages).
    pub fn display(&self) -> String {
        format!("{}/{}/{}", self.origin, self.owner, self.name)
    }
}
