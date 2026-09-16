#![forbid(unsafe_code)]
//! `planeter` — the forge server binary.
//!
//! **M1 preview.** This wires the read path (RFC 003) into a runnable server: create/host repos over
//! sandboxed prikk, browse them through the authorized, honest read API. It uses **in-memory** stores
//! and the **insecure stub** password/token hasher (real Argon2id + SQLite are deferred behind their
//! seams — RFC 002), so it is a **development/preview server, not for production**. Configuration is by
//! environment variable:
//!
//! - `PLANETER_ADDR` — bind address (default `127.0.0.1:8080`)
//! - `PLANETER_REPOS_ROOT` — where hosted repositories live (default `./planeter-repos`)
//! - `PLANETER_CONTENT_ORIGIN` — the isolated origin for raw bytes (default `http://127.0.0.1:8080`)

use std::sync::Arc;

use planeter_auth::{
    Authenticator, InMemoryAccountStore, InMemoryCredentialStore, InsecureStubHasher,
};
use planeter_core::{HostingService, NullAuditSink, ReadService};
use planeter_store::{
    InMemoryMembershipStore, InMemoryRepositoryStore, MembershipStore, RepositoryStore,
};
use planeter_web::{AppState, ContentOrigin};

#[tokio::main]
async fn main() {
    let addr: std::net::SocketAddr = std::env::var("PLANETER_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
        .parse()
        .expect("PLANETER_ADDR must be a socket address");
    let repos_root =
        std::env::var("PLANETER_REPOS_ROOT").unwrap_or_else(|_| "./planeter-repos".to_owned());
    let content_origin =
        std::env::var("PLANETER_CONTENT_ORIGIN").unwrap_or_else(|_| format!("http://{addr}"));

    // In-memory stores + stub hashing — preview only.
    let repo_store: Arc<dyn RepositoryStore> = Arc::new(InMemoryRepositoryStore::new());
    let hosting = Arc::new(HostingService::new(repos_root, repo_store));
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    let read = Arc::new(ReadService::new(
        hosting,
        membership,
        Arc::new(NullAuditSink),
    ));

    let hasher = Arc::new(InsecureStubHasher);
    let auth = Arc::new(Authenticator::new(
        Arc::new(InMemoryAccountStore::new()),
        Arc::new(InMemoryCredentialStore::new()),
        hasher.clone(),
        hasher,
    ));

    let state = AppState {
        read,
        auth,
        content_origin: ContentOrigin::new(content_origin),
    };

    eprintln!("planeter (M1 preview, in-memory, NOT for production) listening on http://{addr}");
    if let Err(e) = planeter_web::serve(state, addr).await {
        eprintln!("planeter: server error: {e}");
        std::process::exit(1);
    }
}
