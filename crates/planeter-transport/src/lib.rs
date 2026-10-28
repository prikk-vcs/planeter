#![forbid(unsafe_code)]
//! `planeter-transport` — planeter upper (surfaces) layer. See RFC 001 (foundations).
//!
//! The transport (RFC 004): the content-addressed [`artifacts`] store and export spool (part 1), and the read side
//! (part 2): the state [`fingerprint`], the [`export`] service that advertises a repository and fetches a verified,
//! spooled, cached artifact, and its [`metrics`]. Nothing here speaks HTTP; the web crate serves it.

pub mod artifacts;
pub mod creation;
pub mod export;
pub mod fingerprint;
pub mod metrics;
pub mod proposals;
pub mod propose;
pub mod recovery;
pub mod reviews;

pub use artifacts::{
    AcceptDir, ArtifactStore, Digest, DirsConfig, DirsConfigError, Removal, Spool, StoreError,
    TransportDirs, UploadWriter,
};
pub use creation::{
    Confirmed, CreateError, CreationConfig, CreationService, LandingOutcome, StageMeta,
    StageSummary, StageUpload, StartupSweep, Uploaded as StageUploaded,
};
pub use export::{
    Advertisement, Artifact, ExportConfig, ExportService, FetchError, Kind, Offer, RefClass,
    RefInfo,
};
pub use metrics::{CacheStatus, FetchRecord, Metrics, Outcome, ProposeOutcome, ProposeRecord};
pub use planeter_prikk::findings::{ArchiveFindings, FoundKey, FoundRef};
pub use proposals::{
    DETAIL_PATCH_LIMIT, Detail, LIST_LIMIT_DEFAULT, LIST_LIMIT_MAX, OBSERVATION_BLOCKS_DEFAULT,
    OBSERVATION_BLOCKS_MAX, OBSERVATION_BLOCKS_MIN, Page, PatchEntry, ProposalConfig,
    ProposalError, ProposalService, REASON_MAX_CHARS, Retention, RevisionView, StateFilter,
    Summary, SweepReport, observation_blocks_from, record_digest,
};
pub use propose::{
    IncompleteCause, MetaError, Plan, ProposeConfig, ProposeError, ProposeMeta, ProposeService,
    Proposed, Submitter, Upload, Uploaded,
};
pub use recovery::{
    Blocker, Cause, Inspection, Note as RecoveryNote, Outcome as RecoveryOutcome,
    Plan as RecoveryPlan, ProcFs, ProcessInfo, ProcessTable, RecoveryConfig, RecoveryError,
    RecoveryResult, RecoveryService, classify_cause, inside_repository,
};
pub use reviews::{
    DecisionView, Limited, NewComment, ReviewConfig, ReviewError, ReviewService, ReviewView,
    THREADS_LIMIT_DEFAULT, THREADS_LIMIT_MAX, Thread, ThreadsPage,
};
