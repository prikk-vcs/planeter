#![forbid(unsafe_code)]
//! `planeter-store` — planeter lower (persistence) layer. See RFC 001 (foundations).
//!
//! The repository/identity records planeter keeps above anonymous prikk ([`model`]), the storage
//! abstraction that persists them ([`store`]), and an in-memory backend ([`memory`]). The production
//! SQLite backend (RFC 001 IQ-4) is a later increment behind the [`store::RepositoryStore`] trait.

pub mod memory;
pub mod model;
pub mod store;

pub use memory::InMemoryRepositoryStore;
pub use model::{Owner, RepoId, RepositoryRecord, Visibility};
pub use store::{RepositoryStore, StoreError};
