//! The OIDC/SSO verification **seam** (RFC 002 T2, `STD-2`). Session sign-in via OAuth 2.0 / OIDC is a
//! standard mechanism, not a redesign — so planeter treats the provider integration as a trait it calls,
//! never home-grown crypto. The concrete verifier (JWKS fetch, signature + claim validation) is a
//! configured integration that drops in behind [`OidcVerifier`]; A0/T2 ships the seam and an
//! unconfigured stub.
//!
//! SAML/LDAP directory integration and TOTP/WebAuthn second factors are further configured integrations;
//! they install behind their own seams and do not block M1 (handoff note). Only the OIDC seam is defined
//! here as the representative case.

/// The identity a verified OIDC id-token asserts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedIdentity {
    /// The provider's stable subject identifier.
    pub subject: String,
    pub email: Option<String>,
}

/// Why OIDC verification failed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OidcError {
    /// No OIDC provider is configured (the A0/T2 default).
    NotConfigured,
    /// The token failed signature or claim validation.
    Invalid,
}

impl std::fmt::Display for OidcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OidcError::NotConfigured => f.write_str("no OIDC provider configured"),
            OidcError::Invalid => f.write_str("invalid OIDC token"),
        }
    }
}

impl std::error::Error for OidcError {}

/// Verifies an OIDC id-token to a [`VerifiedIdentity`]. Implemented by a configured provider client.
pub trait OidcVerifier: Send + Sync {
    fn verify(&self, id_token: &str) -> Result<VerifiedIdentity, OidcError>;
}

/// The default: no provider configured, so every token is refused.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnconfiguredOidc;

impl OidcVerifier for UnconfiguredOidc {
    fn verify(&self, _id_token: &str) -> Result<VerifiedIdentity, OidcError> {
        Err(OidcError::NotConfigured)
    }
}
