#![forbid(unsafe_code)]
//! `planeter-prikk` — planeter's lower (prikk boundary) layer. See RFC 001 (foundations).
//!
//! The **only** crate that touches prikk (RFC 001 `LAY-3`): it drives the prikk CLI as a
//! sandboxed subprocess and parses its `--format json` output into typed values, behind the
//! [`PrikkRepo`] trait so the implementation is swappable (RFC 001 D-7 / OQ-7 — evaluate reusing
//! stikk's `stikk-prikk`). This crate links no prikk crate; the boundary is a process boundary.
//!
//! Design phase: the [`error`] type and the trait/models land in RFC 001 T2; the subprocess driver
//! (T3), the version pin (T3), and the sandbox (T4) follow.

pub mod error;
pub mod model;
pub mod repo;

pub use error::{PrikkError, Result};
pub use repo::PrikkRepo;
