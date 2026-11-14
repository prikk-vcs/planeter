//! `refusal-v1` as prikk 0.56.0 and later give it for a **delivery** (`bundle import`, `archive import`): a rollback, a diverged history or a
//! history that does not connect to the received pointer it would replace is refused before anything is written, unless
//! `--allow-non-fast-forward` is given (**the driver never passes it**); 0.57.0 adds the refusals of a format-8 delivery to an older
//! repository, of a delivery outside the version rule, and of a forged publication signature.
//!
//! The document is on stdout with exit status 1 (the same text is on stderr). It is not the `extract` refusal: its `reason` is always `null`
//! and its `command` is `bundle-import` or `archive-import`. Same reading convention as the rest of the crate: `schema_version` exact, every
//! field required, unknown fields tolerated, an id this driver does not know kept as it came ([`DeliveryRefusal::Other`]).
//! **No flow of planeter reads it yet**: repository creation meets only `no-local-history` (a stage is built from scratch), and what it
//! would show for a refusal is prikk's own `error:` line (`archive_call` in `creation.rs`).

use serde::Deserialize;

use crate::error::{PrikkError, Result};

/// Why prikk refused a delivery, by its id.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "String")]
pub enum DeliveryRefusal {
    /// The delivery is older than the received pointer it would replace.
    Rollback,
    /// The delivery and the received pointer have a common history but neither extends the other.
    Diverged,
    /// The delivery shares no history with the received pointer.
    DoesNotConnect,
    /// 0.57.0: a format-8 bundle or archive, delivered to a repository of an older format.
    RepositoryFormatTooOld,
    /// 0.57.0: a delivery outside the version rule.
    OutsideVersionRule,
    /// 0.57.0: a held Block or RefState re-sent with a bad MAINTAINER signature (also for a new one).
    ForgedPublicationSignature,
    /// An id this driver does not know (a later prikk), kept as it came.
    Other(String),
}

impl DeliveryRefusal {
    /// The ids this driver knows, in the order above.
    pub const KNOWN: [&'static str; 6] = [
        "rollback",
        "diverged",
        "does-not-connect",
        "repository-format-too-old",
        "outside-version-rule",
        "forged-publication-signature",
    ];

    /// prikk's id for it.
    pub fn id(&self) -> &str {
        match self {
            DeliveryRefusal::Rollback => "rollback",
            DeliveryRefusal::Diverged => "diverged",
            DeliveryRefusal::DoesNotConnect => "does-not-connect",
            DeliveryRefusal::RepositoryFormatTooOld => "repository-format-too-old",
            DeliveryRefusal::OutsideVersionRule => "outside-version-rule",
            DeliveryRefusal::ForgedPublicationSignature => "forged-publication-signature",
            DeliveryRefusal::Other(s) => s,
        }
    }
}

impl From<String> for DeliveryRefusal {
    fn from(s: String) -> Self {
        match s.as_str() {
            "rollback" => DeliveryRefusal::Rollback,
            "diverged" => DeliveryRefusal::Diverged,
            "does-not-connect" => DeliveryRefusal::DoesNotConnect,
            "repository-format-too-old" => DeliveryRefusal::RepositoryFormatTooOld,
            "outside-version-rule" => DeliveryRefusal::OutsideVersionRule,
            "forged-publication-signature" => DeliveryRefusal::ForgedPublicationSignature,
            _ => DeliveryRefusal::Other(s),
        }
    }
}

/// A refused delivery.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ImportRefusal {
    /// `bundle-import` or `archive-import`.
    pub command: String,
    pub refusal: DeliveryRefusal,
    /// Always `null` for a delivery refusal; required to be present.
    #[serde(deserialize_with = "present_null_or_string")]
    pub reason: Option<String>,
    /// prikk's own English text, verbatim. Not user-facing.
    pub message: String,
}

fn present_null_or_string<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<String>, D::Error> {
    Option::<String>::deserialize(d)
}

/// Read the stdout of a refused `archive import` or `bundle import`. The command must be one of the two.
pub fn parse_import_refusal(stdout: &str) -> Result<ImportRefusal> {
    let r: ImportRefusal = crate::cli::parse_schema_json(stdout, "refusal-v1")?;
    if r.command != "archive-import" && r.command != "bundle-import" {
        return Err(PrikkError::Parse(format!(
            "refusal-v1 for the command {:?}, not archive-import or bundle-import",
            r.command
        )));
    }
    Ok(r)
}
