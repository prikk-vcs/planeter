#![forbid(unsafe_code)]
//! `planeter-store` — planeter lower (persistence) layer. See RFC 001 (foundations).
//!
//! The repository/identity records planeter keeps above anonymous prikk ([`model`]), the storage
//! abstraction that persists them ([`store`]), an in-memory backend ([`memory`]) for tests/dev, and the
//! production **SQLite** backend ([`sqlite`], RFC 001 IQ-4) behind the same traits.

pub mod membership;
pub mod memory;
pub mod model;
pub mod sqlite;
pub mod store;

pub use membership::{
    InMemoryMembershipStore, MembershipStore, OrgId, OrgRole, Role, Subject, TeamId, UserId,
};
pub use memory::InMemoryRepositoryStore;
pub use model::{Owner, RepoId, RepositoryRecord, Visibility};
pub use sqlite::{SqliteDb, SqliteMembershipStore, SqliteRepositoryStore};
pub use store::{RepositoryStore, StoreError};
