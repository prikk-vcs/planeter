#![forbid(unsafe_code)]
//! `planeter-store` — planeter lower (persistence) layer. See RFC 001 (foundations).
//!
//! The repository/identity records planeter keeps above anonymous prikk ([`model`]), the storage
//! abstraction that persists them ([`store`]), an in-memory backend ([`memory`]) for tests/dev, and the
//! production **SQLite** backend ([`sqlite`], RFC 001 IQ-4) behind the same traits.

pub mod change;
pub mod lease;
pub mod membership;
pub mod memory;
pub mod model;
pub mod revision;
pub mod sqlite;
pub mod store;

pub use change::{
    ChangeRecord, ChangeState, ChangeStore, ClaimOutcome, FailPoint, InMemoryChangeStore,
    IntentRecord, IntentState, RepoFlags, RevisionRecord, SqliteChangeStore, Usage, pack_ids,
    unpack_ids,
};
pub use lease::{HolderRecord, InMemoryLeaseRecords, LeaseRecordStore, SqliteLeaseRecords};
pub use membership::{
    InMemoryMembershipStore, MembershipStore, OrgId, OrgRole, Role, Subject, TeamId, UserId,
};
pub use memory::InMemoryRepositoryStore;
pub use model::{
    Owner, ProposalPolicy, RepoId, RepositoryRecord, UnknownProposalPolicy, Visibility,
};
pub use revision::{InMemoryRevisions, RevisionStore, SqliteRevisions};
pub use sqlite::{SqliteDb, SqliteMembershipStore, SqliteRepositoryStore};
pub use store::{RepositoryStore, StoreError};
