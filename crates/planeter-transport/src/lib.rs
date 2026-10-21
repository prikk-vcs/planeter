#![forbid(unsafe_code)]
//! `planeter-transport` — planeter upper (surfaces) layer. See RFC 001 (foundations).
//!
//! The transport's behaviour (keyless clone/push, RFC 004) arrives at M2, held until prikk ships RFC 155 then
//! RFC 154. What exists now are its **foundations** (RFC 004 part 1): the content-addressed [`artifacts`] store
//! and export spool with their configuration. Nothing here is reachable from the network.

pub mod artifacts;

pub use artifacts::{
    ArtifactStore, Digest, DirsConfig, DirsConfigError, Spool, StoreError, TransportDirs,
};
