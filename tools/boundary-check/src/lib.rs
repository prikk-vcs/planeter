#![forbid(unsafe_code)]
//! `planeter-boundary-check` — the layering gate (RFC 001 `LAY-1`).
//!
//! The check itself lives in `tests/layering.rs`: dependencies point downward only, so no lower/core
//! crate may declare a dependency on an upper crate. This crate carries no product code.
