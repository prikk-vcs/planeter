//! The rules of creating a repository (RFC 004 part 7): who may create where, what a name may be, how many a namespace may
//! hold, and what the creator must **type** before a maintainer's key is trusted.
//!
//! Everything here is pure: it reads the state it is given and decides. The sequence that writes (reserve, `init`, trust,
//! import, verify, ready) is the hosting and transport layers'; these are the decisions it asks for.
//!
//! # Names
//!
//! A repository name is lowercase ASCII (`a-z`, `0-9`, `.`, `_`, `-`), starts with a letter or digit, is at most 64 long, has
//! no `..`, does not end with `.` or `.git`, and is not **reserved**. Lowercase only is deliberate: it makes a case collision
//! (`App` and `app` on a case-insensitive disk or in a person's eye) impossible rather than something to detect. What is
//! reserved is not listed here: the web layer derives it from its routes (the first segment of every static route), and
//! passes the test in.
//!
//! # The fingerprint
//!
//! prikk gives a maintainer key two names, a short id (`ed25519-` and 16 hexadecimal characters) and the public key (64
//! hexadecimal characters, what `prikk key public` prints). The **fingerprint** this forge shows and asks for is the whole
//! public key: it is the one thing a maintainer can read off their own machine, which is the only way a creator can compare
//! it through a channel the archive does not control. A hash of it would be something nobody else can produce.
//!
//! # Adoption
//!
//! Trust is the creator's act, never the archive's. [`resolve_adoption`] checks the typed fingerprints against the keys the
//! archive carries and refuses unless **every** carried key is either matched or explicitly left out, naming the key that
//! failed and revealing nothing of the expected value.

use planeter_store::{MembershipStore, OrgId, Owner, ProposalPolicy, UserId, Visibility};

use crate::authorize::{AccessContext, Action, OrgResource, Principal, Resource, authorize};

/// The longest repository name.
pub const NAME_MAX: usize = 64;
/// The longest owner handle accepted in a request.
pub const HANDLE_MAX: usize = 128;
/// The default most repositories one user may own (counting the ones being created).
pub const DEFAULT_USER_QUOTA: u64 = 50;
/// The default most repositories one organization may own.
pub const DEFAULT_ORG_QUOTA: u64 = 500;
/// The default most creations one principal may start in an hour (empty creations and staged archives together).
pub const DEFAULT_CREATIONS_PER_HOUR: u32 = 10;

// ---------------------------------------------------------------------------------------------------- names

/// Why a name or a handle is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong,
    /// A letter that is not lowercase ASCII (names only).
    Uppercase,
    /// A character outside the allowed set (including `/`, `\`, NUL, spaces, control characters and non-ASCII).
    BadCharacter,
    /// Does not start with a letter or a digit.
    BadStart,
    /// `..`, a trailing `.`, or a trailing `.git`.
    Dotted,
    /// The name belongs to the server (a path it answers on).
    Reserved,
}

impl NameError {
    /// A stable word for a response body and a log.
    pub fn reason(self) -> &'static str {
        match self {
            NameError::Empty => "empty",
            NameError::TooLong => "too_long",
            NameError::Uppercase => "uppercase",
            NameError::BadCharacter => "bad_character",
            NameError::BadStart => "bad_start",
            NameError::Dotted => "dotted",
            NameError::Reserved => "reserved",
        }
    }
}

/// Check a repository `name`. `reserved` says whether a (lowercase) word belongs to the server.
pub fn check_repo_name(name: &str, reserved: &dyn Fn(&str) -> bool) -> Result<(), NameError> {
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name.len() > NAME_MAX {
        return Err(NameError::TooLong);
    }
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            return Err(NameError::Uppercase);
        }
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')) {
            return Err(NameError::BadCharacter);
        }
    }
    if !name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit()) {
        return Err(NameError::BadStart);
    }
    if name.contains("..") || name.ends_with('.') || name.ends_with(".git") {
        return Err(NameError::Dotted);
    }
    if reserved(name) {
        return Err(NameError::Reserved);
    }
    Ok(())
}

/// Check an owner `handle` named in a request. This is a syntax check only (it can say nothing about whether the owner
/// exists, and must not): the characters of a path segment that cannot climb out of one.
pub fn check_owner_handle(handle: &str, reserved: &dyn Fn(&str) -> bool) -> Result<(), NameError> {
    if handle.is_empty() {
        return Err(NameError::Empty);
    }
    if handle.len() > HANDLE_MAX {
        return Err(NameError::TooLong);
    }
    if !handle
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@' | '+' | '~'))
    {
        return Err(NameError::BadCharacter);
    }
    if !handle.starts_with(|c: char| c.is_ascii_alphanumeric()) {
        return Err(NameError::BadStart);
    }
    if handle.contains("..") || handle.ends_with('.') {
        return Err(NameError::Dotted);
    }
    if reserved(&handle.to_ascii_lowercase()) {
        return Err(NameError::Reserved);
    }
    Ok(())
}

/// The word of a [`Visibility`].
pub fn visibility_word(v: Visibility) -> &'static str {
    match v {
        Visibility::Public => "public",
        Visibility::Internal => "internal",
        Visibility::Private => "private",
    }
}

/// Parse a visibility word. Anything else is refused, never defaulted.
pub fn parse_visibility(word: &str) -> Option<Visibility> {
    match word {
        "public" => Some(Visibility::Public),
        "internal" => Some(Visibility::Internal),
        "private" => Some(Visibility::Private),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------------- who may create where

/// Why a creation target was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetRefusal {
    /// The caller is not a signed-in user.
    NotSignedIn,
    /// The caller may not create there: someone else's namespace, an organization they are not in, or one that does not
    /// exist — **not told apart**, so the refusal is no oracle for which organizations exist.
    Refused,
}

/// Where `principal` may create a repository named for `handle`: their **own namespace** (`handle` is their user id), or
/// an organization where `authorize(CreateRepo)` allows (any member). The credential's own ceiling applies as everywhere.
pub fn creation_target(
    principal: &Principal,
    handle: &str,
    membership: &dyn MembershipStore,
    now_unix: u64,
) -> Result<Owner, TargetRefusal> {
    let Some(user) = principal.user() else {
        return Err(if principal.is_authenticated() {
            TargetRefusal::Refused
        } else {
            TargetRefusal::NotSignedIn
        });
    };
    let scope_permits = || match principal.scope() {
        None => true,
        Some(scope) => !scope.is_expired(now_unix) && scope.permits_action(Action::CreateRepo),
    };
    if handle == user.as_str() {
        return if scope_permits() {
            Ok(Owner::User(handle.to_owned()))
        } else {
            Err(TargetRefusal::Refused)
        };
    }
    let org_role = membership
        .org_role(user, &OrgId::new(handle))
        .map_err(|_| TargetRefusal::Refused)?;
    let ctx = AccessContext {
        repo_role: None,
        org_role,
        now_unix,
    };
    let resource = Resource::Org(OrgResource {
        owner: Owner::Org(handle.to_owned()),
    });
    if authorize(principal, Action::CreateRepo, &resource, &ctx).is_allowed() {
        Ok(Owner::Org(handle.to_owned()))
    } else {
        Err(TargetRefusal::Refused)
    }
}

/// The most repositories `owner` may hold, given the operator's two limits.
pub fn quota_for(owner: &Owner, user_limit: u64, org_limit: u64) -> u64 {
    match owner {
        Owner::User(_) => user_limit,
        Owner::Org(_) => org_limit,
    }
}

/// Whether one more repository fits: `owned` (ready and being created) is below `limit`.
pub fn fits_quota(owned: u64, limit: u64) -> bool {
    owned < limit
}

/// What a creation asks for, validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRepo {
    pub owner: String,
    pub name: String,
    pub visibility: Visibility,
    pub proposals: ProposalPolicy,
}

/// Why a request's fields were refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestError {
    Owner(NameError),
    Name(NameError),
    /// `visibility` is not `public`, `internal` or `private`.
    Visibility,
    /// `proposals` is not `members` or `any-authenticated`.
    Proposals,
}

impl NewRepo {
    /// Validate the four fields of a request. `proposals` defaults to `members`.
    pub fn parse(
        owner: &str,
        name: &str,
        visibility: &str,
        proposals: Option<&str>,
        reserved: &dyn Fn(&str) -> bool,
    ) -> Result<NewRepo, RequestError> {
        check_owner_handle(owner, reserved).map_err(RequestError::Owner)?;
        check_repo_name(name, reserved).map_err(RequestError::Name)?;
        let visibility = parse_visibility(visibility).ok_or(RequestError::Visibility)?;
        let proposals = match proposals {
            None => ProposalPolicy::Members,
            Some(p) => ProposalPolicy::parse(p).map_err(|_| RequestError::Proposals)?,
        };
        Ok(NewRepo {
            owner: owner.to_owned(),
            name: name.to_owned(),
            visibility,
            proposals,
        })
    }
}

// ---------------------------------------------------------------------------------------------------- fingerprints and adoption

/// Normalise what a person typed or pasted as a fingerprint: spaces, tabs, line breaks and `:` are dropped (the page shows
/// the key in groups; a paste keeps them), letters are lowercased, and the result must be exactly 64 hexadecimal characters.
/// Anything else is `None`: a prefix, a short id or another key's text is not a fingerprint.
pub fn normalize_fingerprint(typed: &str) -> Option<String> {
    let mut out = String::with_capacity(64);
    for c in typed.chars() {
        if c.is_whitespace() || c == ':' {
            continue;
        }
        if !c.is_ascii_hexdigit() {
            return None;
        }
        out.push(c.to_ascii_lowercase());
        if out.len() > 64 {
            return None;
        }
    }
    (out.len() == 64).then_some(out)
}

/// A fingerprint in groups of eight for reading and copying (`388c758c 92be70ee …`). Input that is not 64 hexadecimal
/// characters is returned as it is.
pub fn group_fingerprint(hex: &str) -> String {
    if hex.len() != 64 || !hex.is_ascii() {
        return hex.to_owned();
    }
    hex.as_bytes()
        .chunks(8)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(" ")
}

/// One key an archive carries, as the confirmation sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carried {
    pub key_id: String,
    /// The full public key (64 lowercase hexadecimal characters): the fingerprint.
    pub fingerprint: String,
}

/// What the creator sent about the carried keys.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdoptionRequest {
    /// `(key id, the fingerprint the creator typed)` for each key to trust.
    pub adopt: Vec<(String, String)>,
    /// The key ids the creator explicitly does **not** trust.
    pub leave_out: Vec<String>,
    /// Trust none of them (a shorthand for leaving every carried key out).
    pub no_adopt: bool,
}

/// Why an adoption request is refused. Never carries an expected fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdoptionError {
    /// `no_adopt` together with keys to adopt or to leave out.
    Conflicting,
    /// A key id the archive does not carry.
    UnknownKey { key_id: String },
    /// A key named twice (or both adopted and left out).
    Duplicate { key_id: String },
    /// The typed text is not a fingerprint (not 64 hexadecimal characters).
    Malformed { key_id: String },
    /// The typed fingerprint is not the carried key's.
    Mismatch { key_id: String },
    /// Carried keys that are neither adopted nor left out.
    Unaddressed { key_ids: Vec<String> },
}

/// The keys to trust and the keys left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adoption {
    pub adopted: Vec<Carried>,
    pub left_out: Vec<Carried>,
}

/// Check the creator's answer against the carried keys. Every carried key must be **adopted** (its typed fingerprint equals
/// the carried one, whole, ignoring case and grouping) or **explicitly left out**; `no_adopt` leaves all of them out.
pub fn resolve_adoption(
    carried: &[Carried],
    req: &AdoptionRequest,
) -> Result<Adoption, AdoptionError> {
    if req.no_adopt && (!req.adopt.is_empty() || !req.leave_out.is_empty()) {
        return Err(AdoptionError::Conflicting);
    }
    let known = |id: &str| carried.iter().find(|c| c.key_id == id);
    let mut seen: Vec<&str> = Vec::new();
    let mut adopted = Vec::new();
    for (key_id, typed) in &req.adopt {
        let key = known(key_id).ok_or_else(|| AdoptionError::UnknownKey {
            key_id: key_id.clone(),
        })?;
        if seen.contains(&key_id.as_str()) {
            return Err(AdoptionError::Duplicate {
                key_id: key_id.clone(),
            });
        }
        seen.push(key_id);
        let Some(typed) = normalize_fingerprint(typed) else {
            return Err(AdoptionError::Malformed {
                key_id: key_id.clone(),
            });
        };
        if typed != key.fingerprint {
            return Err(AdoptionError::Mismatch {
                key_id: key_id.clone(),
            });
        }
        adopted.push(key.clone());
    }
    let mut left_out = Vec::new();
    for key_id in &req.leave_out {
        let key = known(key_id).ok_or_else(|| AdoptionError::UnknownKey {
            key_id: key_id.clone(),
        })?;
        if seen.contains(&key_id.as_str()) {
            return Err(AdoptionError::Duplicate {
                key_id: key_id.clone(),
            });
        }
        seen.push(key_id);
        left_out.push(key.clone());
    }
    if req.no_adopt {
        left_out = carried.to_vec();
    } else {
        let missing: Vec<String> = carried
            .iter()
            .filter(|c| !seen.contains(&c.key_id.as_str()))
            .map(|c| c.key_id.clone())
            .collect();
        if !missing.is_empty() {
            return Err(AdoptionError::Unaddressed { key_ids: missing });
        }
    }
    Ok(Adoption { adopted, left_out })
}

/// Whether `user` is the namespace's own user (for the page, which shows the personal namespace first).
pub fn is_own_namespace(user: &UserId, handle: &str) -> bool {
    user.as_str() == handle
}
