#![forbid(unsafe_code)]
//! `planeter-transport` — planeter upper (surfaces) layer. See RFC 001 (foundations).
//!
//! The transport (RFC 004): the content-addressed [`artifacts`] store and export spool (part 1), and the read side
//! (part 2): the state [`fingerprint`], the [`export`] service that advertises a repository and fetches a verified,
//! spooled, cached artifact, and its [`metrics`]. Nothing here speaks HTTP; the web crate serves it.

pub mod artifacts;
pub mod changes;
pub mod creation;
pub mod delivery;
#[cfg(test)]
mod exec_wait;
pub mod export;
pub mod fingerprint;
pub mod keys;
pub mod metrics;
pub mod proposals;
pub mod propose;
pub mod recovery;
pub mod reviews;

pub use artifacts::{
    AcceptDir, ArtifactStore, Digest, DirsConfig, DirsConfigError, Removal, Spool, StoreError,
    TransportDirs, UploadWriter,
};
pub use changes::engine::{
    ChangesConfig, ChangesError, ChangesService, Flight, MAX_WAITERS, Reason as ChangesReason,
    TooManyWaiters,
};
pub use creation::{
    Confirmed, CreateError, CreationConfig, CreationService, LandingOutcome, StageMeta,
    StageSummary, StageUpload, StartupSweep, Uploaded as StageUploaded,
};
pub use delivery::{
    DeliveryConfig, DeliveryError, DeliveryService, DeliveryUpload, DeliveryView, Installed,
    Reconciled, Report as DeliveryReport, Uploaded as DeliveryUploaded,
};
pub use export::{
    Advertisement, Artifact, ExportConfig, ExportService, FetchError, Kind, Offer, RefClass,
    RefInfo,
};
pub use keys::{
    BackfillReport, Backfilled, KeyService, KeysError, RepoKey, Source as KeySourceView,
    detail_for_record, join_keys, record_creation as record_creation_keys, repo_keys,
};
pub use metrics::{CacheStatus, FetchRecord, Metrics, Outcome, ProposeOutcome, ProposeRecord};
pub use planeter_prikk::findings::{ArchiveFindings, FoundKey, FoundRef};
pub use proposals::{
    DETAIL_PATCH_LIMIT, Detail, FOLLOWED_REF, LIST_LIMIT_DEFAULT, LIST_LIMIT_MAX,
    OBSERVATION_BLOCKS_DEFAULT, OBSERVATION_BLOCKS_MAX, OBSERVATION_BLOCKS_MIN, Page, PatchEntry,
    ProposalConfig, ProposalError, ProposalService, REASON_MAX_CHARS, Retention, RevisionView,
    StateFilter, Summary, SweepReport, observation_blocks_from, record_digest,
};
pub use propose::{
    IncompleteCause, MetaError, Plan, ProposeConfig, ProposeError, ProposeMeta, ProposeService,
    Proposed, Submitter, Upload, Uploaded, refused_on_evidence,
};
pub use recovery::{
    Blocker, Cause, DeliveryCompletion, Inspection, Note as RecoveryNote,
    Outcome as RecoveryOutcome, Plan as RecoveryPlan, ProcFs, ProcessInfo, ProcessTable,
    RecoveryConfig, RecoveryError, RecoveryResult, RecoveryService, classify_cause,
    inside_repository,
};
pub use reviews::{
    DecisionView, Limited, NewComment, ReviewConfig, ReviewError, ReviewService, ReviewView,
    THREADS_LIMIT_DEFAULT, THREADS_LIMIT_MAX, Thread, ThreadsPage,
};
