//! `planeter-client`: a person's own prikk, talking to a planeter forge (RFC 004 part 9). T0 of the handoff: the dependency
//! set builds on Linux, macOS and Windows; the commands arrive in the following phases.
#![forbid(unsafe_code)]

fn main() {
    // The one provider the program uses, named once (D-50). Installing it twice is not an error the program can have.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    println!("planeter-client {}", env!("CARGO_PKG_VERSION"));
}
