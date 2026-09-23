//! OpenSSH **ed25519** public-key parsing and SHA-256 fingerprints (RFC 002 T2). A registered SSH key
//! maps a fingerprint to a user; signature *verification* is transport's job (RFC 004).
//!
//! Hand-rolled on purpose (owner-approved review, 2026-09-23): the `ssh-key` crate locks the unfixable
//! RUSTSEC-2023-0071 `rsa` crate into `Cargo.lock` for an algorithm planeter never uses. ed25519 is the
//! ecosystem's key type (prikk signs with ed25519); RSA/ECDSA SSH keys are **unsupported**, refused with
//! [`SshKeyError::UnsupportedAlgorithm`] rather than silently accepted.
//!
//! The wire format (RFC 4253 §6.6 / the OpenSSH key blob) is `string "ssh-ed25519" || string key(32)`;
//! the fingerprint is `SHA256:` + unpadded base64 of SHA-256 over that blob — exactly what
//! `ssh-keygen -lf` prints, which the tests lock to.

use base64ct::{Base64, Base64Unpadded, Encoding as _};
use sha2::Digest as _;

use planeter_core::Scope;
use planeter_store::UserId;

use crate::credential::SshKeyRecord;

const ED25519: &str = "ssh-ed25519";

/// A parsed ed25519 OpenSSH public key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshPublicKey {
    /// The 32-byte ed25519 public key.
    pub key: [u8; 32],
    /// The optional trailing comment (e.g. `user@host`).
    pub comment: Option<String>,
    /// The raw wire blob (what the fingerprint is computed over).
    blob: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SshKeyError {
    /// Not `ssh-ed25519` — RSA/ECDSA/others are unsupported.
    UnsupportedAlgorithm(String),
    /// The line or its base64 blob is malformed.
    Malformed,
}

impl std::fmt::Display for SshKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SshKeyError::UnsupportedAlgorithm(a) => {
                write!(f, "unsupported SSH key algorithm {a:?} (only ssh-ed25519)")
            }
            SshKeyError::Malformed => f.write_str("malformed OpenSSH public key"),
        }
    }
}

impl std::error::Error for SshKeyError {}

impl SshPublicKey {
    /// Parse one `authorized_keys`-style line: `ssh-ed25519 <base64> [comment]`.
    pub fn parse_openssh(line: &str) -> Result<Self, SshKeyError> {
        let mut parts = line.split_whitespace();
        let algo = parts.next().ok_or(SshKeyError::Malformed)?;
        if algo != ED25519 {
            return Err(SshKeyError::UnsupportedAlgorithm(algo.to_owned()));
        }
        let b64 = parts.next().ok_or(SshKeyError::Malformed)?;
        let comment = {
            let rest: Vec<&str> = parts.collect();
            if rest.is_empty() {
                None
            } else {
                Some(rest.join(" "))
            }
        };
        let blob = Base64::decode_vec(b64).map_err(|_| SshKeyError::Malformed)?;

        // Wire: string(algo) || string(key)
        let (name, rest) = take_string(&blob).ok_or(SshKeyError::Malformed)?;
        if name != ED25519.as_bytes() {
            return Err(SshKeyError::Malformed);
        }
        let (key, rest) = take_string(rest).ok_or(SshKeyError::Malformed)?;
        if key.len() != 32 || !rest.is_empty() {
            return Err(SshKeyError::Malformed);
        }
        let mut k = [0u8; 32];
        k.copy_from_slice(key);
        Ok(Self {
            key: k,
            comment,
            blob,
        })
    }

    /// The `SHA256:<base64>` fingerprint, as `ssh-keygen -lf` prints it.
    pub fn fingerprint_sha256(&self) -> String {
        let digest = sha2::Sha256::digest(&self.blob);
        format!("SHA256:{}", Base64Unpadded::encode_string(&digest))
    }
}

/// Read one RFC 4251 `string` (u32 big-endian length + bytes) from the front of `buf`.
fn take_string(buf: &[u8]) -> Option<(&[u8], &[u8])> {
    let len = u32::from_be_bytes(buf.get(..4)?.try_into().ok()?) as usize;
    let rest = buf.get(4..)?;
    let (s, tail) = rest.split_at_checked(len)?;
    Some((s, tail))
}

impl SshKeyRecord {
    /// Build a credential record from an OpenSSH public-key line: parse, fingerprint, bind to `user`.
    pub fn from_openssh(line: &str, user: UserId, scope: Scope) -> Result<Self, SshKeyError> {
        let key = SshPublicKey::parse_openssh(line)?;
        Ok(Self {
            fingerprint: key.fingerprint_sha256(),
            user,
            scope,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Generated with `ssh-keygen -t ed25519`; fingerprint from `ssh-keygen -lf`.
    const LINE: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOCvAStfHKilNLoC9wIvNE5U5dduSt5xjZNyDH6tNXf8 planeter-test";
    const FP: &str = "SHA256:GXlQmqnqnl7v+ISvUQvG6T0Y9qQl/zJ9ydwrmZScpuA";

    #[test]
    fn parses_and_fingerprints_like_ssh_keygen() {
        let k = SshPublicKey::parse_openssh(LINE).unwrap();
        assert_eq!(k.comment.as_deref(), Some("planeter-test"));
        assert_eq!(k.fingerprint_sha256(), FP);
        // Without a comment, same key, same fingerprint.
        let no_comment = LINE.rsplit_once(' ').unwrap().0;
        assert_eq!(
            SshPublicKey::parse_openssh(no_comment)
                .unwrap()
                .fingerprint_sha256(),
            FP
        );
    }

    #[test]
    fn rejects_other_algorithms_and_garbage() {
        assert!(matches!(
            SshPublicKey::parse_openssh("ssh-rsa AAAAB3NzaC1yc2E= x"),
            Err(SshKeyError::UnsupportedAlgorithm(_))
        ));
        assert_eq!(SshPublicKey::parse_openssh(""), Err(SshKeyError::Malformed));
        assert_eq!(
            SshPublicKey::parse_openssh("ssh-ed25519 !!!notbase64!!!"),
            Err(SshKeyError::Malformed)
        );
        // Valid base64 but the wrong wire contents.
        assert_eq!(
            SshPublicKey::parse_openssh("ssh-ed25519 AAAA"),
            Err(SshKeyError::Malformed)
        );
    }

    #[test]
    fn record_from_openssh_binds_fingerprint_to_user() {
        use planeter_core::{RepoScope, ScopeAccess};
        let rec = SshKeyRecord::from_openssh(
            LINE,
            UserId::new("alice"),
            Scope {
                access: ScopeAccess::Full,
                repos: RepoScope::All,
                expires_at: None,
            },
        )
        .unwrap();
        assert_eq!(rec.fingerprint, FP);
        assert_eq!(rec.user, UserId::new("alice"));
    }
}
