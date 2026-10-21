//! The repository/identity records planeter keeps *above* anonymous prikk (RFC 001 D-5): a prikk
//! repository has no name, owner or URL of its own (RFC 145 §7), so planeter assigns them here. The
//! bytes on disk are keyed by the stable [`RepoId`]; owner/name are metadata a rename/transfer changes
//! without moving anything (RFC 001 D-4/D-5).

use std::path::PathBuf;

/// A stable, opaque internal id for a hosted repository. The on-disk `.prikk` is laid out by this id,
/// so renaming or transferring a repository never moves bytes. Allocated by the hosting layer
/// (planeter-core, RFC 001 D-6); the store only persists it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoId(String);

impl RepoId {
    /// Wrap an already-allocated id.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RepoId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The owner of a repository: a user or an organization, referenced by its forge-level handle. (The
/// full account/org model is RFC 002; here an owner is just the reference the URL is built from.)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Owner {
    User(String),
    Org(String),
}

impl Owner {
    /// The owner's handle (the `<owner>` segment of `owner/name`).
    pub fn handle(&self) -> &str {
        match self {
            Owner::User(h) | Owner::Org(h) => h,
        }
    }
}

/// Repository visibility. Stored here as data; *enforced* by the authorization service (RFC 002 D-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Internal,
    Private,
}

/// Who may **propose** a change to a repository (RFC 004 D-3, RFC 005 D-9): the repository setting `proposals`.
///
/// The stored and displayed forms are the two words `members` and `any-authenticated`; anything else is
/// **refused**, never defaulted ([`ProposalPolicy::parse`]). The default is `members`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProposalPolicy {
    /// Only principals with `write` or above may propose.
    #[default]
    Members,
    /// Any authenticated principal who can read the repository may propose (a public project taking
    /// contributions from strangers).
    AnyAuthenticated,
}

impl ProposalPolicy {
    /// The stable text form.
    pub fn as_str(self) -> &'static str {
        match self {
            ProposalPolicy::Members => "members",
            ProposalPolicy::AnyAuthenticated => "any-authenticated",
        }
    }

    /// Parse the text form. An unknown value is an error, not a default: a typo in a setting must not quietly
    /// open or close contributions.
    pub fn parse(text: &str) -> Result<Self, UnknownProposalPolicy> {
        match text {
            "members" => Ok(ProposalPolicy::Members),
            "any-authenticated" => Ok(ProposalPolicy::AnyAuthenticated),
            other => Err(UnknownProposalPolicy(other.to_owned())),
        }
    }
}

/// A `proposals` value that is not `members` or `any-authenticated`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownProposalPolicy(pub String);

impl std::fmt::Display for UnknownProposalPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unknown proposals setting {:?} (expected `members` or `any-authenticated`)",
            self.0
        )
    }
}

impl std::error::Error for UnknownProposalPolicy {}

/// A hosted repository's forge-level record. `repo_id` and `path` are stable for the repository's life;
/// `owner`/`name` may change (rename/transfer) without touching either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryRecord {
    pub repo_id: RepoId,
    pub owner: Owner,
    pub name: String,
    pub visibility: Visibility,
    /// Unix seconds at creation.
    pub created_at: u64,
    /// The prikk on-disk format version this repo is hosted in (RFC 009 durability; may be unknown
    /// until the repo is first read).
    pub prikk_format_version: Option<String>,
    /// Who may propose a change (RFC 005 D-9); `members` unless the repository's administrator says otherwise.
    pub proposals: ProposalPolicy,
    /// The directory containing this repository's `.prikk`. Keyed by `repo_id` (RFC 001 D-4), so it is
    /// invariant under rename/transfer.
    pub path: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_proposals_parser_accepts_the_two_words_and_refuses_everything_else() {
        assert_eq!(
            ProposalPolicy::parse("members"),
            Ok(ProposalPolicy::Members)
        );
        assert_eq!(
            ProposalPolicy::parse("any-authenticated"),
            Ok(ProposalPolicy::AnyAuthenticated)
        );
        for bad in [
            "",
            "Members",
            "MEMBERS",
            "any",
            "anyone",
            "any_authenticated",
            " members",
            "members ",
            "none",
            "true",
        ] {
            assert_eq!(
                ProposalPolicy::parse(bad),
                Err(UnknownProposalPolicy(bad.to_owned())),
                "{bad:?} must be refused, not defaulted"
            );
        }
        // The text form round-trips, and the default is `members`.
        for p in [ProposalPolicy::Members, ProposalPolicy::AnyAuthenticated] {
            assert_eq!(ProposalPolicy::parse(p.as_str()), Ok(p));
        }
        assert_eq!(ProposalPolicy::default(), ProposalPolicy::Members);
    }
}
