//! `planeter-client`: a person's own prikk, talking to a planeter server over https (RFC 004 part 9, D-10).
//!
//! T1 of the handoff: the HTTP core, the bearer token, the repository address, the message catalog and the exit codes. The
//! commands arrive with T2 (propose) and T3 (clone and fetch).
#![forbid(unsafe_code)]

pub mod catalog;
pub mod exit;
pub mod http;
pub mod token;
pub mod url;
