#![forbid(unsafe_code)]
//! `planeter-transport` — planeter upper (surfaces) layer. See RFC 001 (foundations).
//!
//! The transport (RFC 004): the content-addressed [`artifacts`] store and export spool (part 1), and the read side
//! (part 2): the state [`fingerprint`], the [`export`] service that advertises a repository and fetches a verified,
//! spooled, cached artifact, and its [`metrics`]. Nothing here speaks HTTP; the web crate serves it.

pub mod artifacts;
pub mod export;
pub mod fingerprint;
pub mod metrics;

pub use artifacts::{
    ArtifactStore, Digest, DirsConfig, DirsConfigError, Spool, StoreError, TransportDirs,
};
pub use export::{
    Advertisement, Artifact, ExportConfig, ExportService, FetchError, Kind, Offer, RefClass,
    RefInfo,
};
pub use metrics::{CacheStatus, FetchRecord, Metrics, Outcome};
