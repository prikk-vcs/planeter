#![forbid(unsafe_code)]
//! `planeter` — the forge server binary.
//!
//! **M1, pre-tag.** This wires the read path (RFC 003) into a runnable server: create/host repos over
//! sandboxed prikk, browse them through the authorized, honest read API, with the production
//! **Argon2id** password hasher and constant-time token hasher (RFC 002 seams, wired 2026-09-23). It
//! still uses **in-memory** stores (SQLite is the remaining pre-tag increment), so state does not
//! survive a restart — **not yet for production**. Configuration is by environment variable:
//!
//! - `PLANETER_ADDR` — bind address (default `127.0.0.1:8080`)
//! - `PLANETER_REPOS_ROOT` — where hosted repositories live (default `./planeter-repos`)
//! - `PLANETER_CONTENT_ORIGIN` — the isolated origin for raw bytes (default `http://127.0.0.1:8080`)

use std::sync::Arc;

use planeter_auth::{
    Argon2idHasher, Authenticator, InMemoryAccountStore, InMemoryCredentialStore, Sha256TokenHasher,
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

    // In-memory stores (SQLite is the remaining pre-tag increment); production hashers.
    let repo_store: Arc<dyn RepositoryStore> = Arc::new(InMemoryRepositoryStore::new());
    let hosting = Arc::new(HostingService::new(repos_root, repo_store));
    let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::new());
    let read = Arc::new(ReadService::new(
        hosting,
        membership,
        Arc::new(NullAuditSink),
    ));

    let auth = Arc::new(Authenticator::new(
        Arc::new(InMemoryAccountStore::new()),
        Arc::new(InMemoryCredentialStore::new()),
        Arc::new(Argon2idHasher),
        Arc::new(Sha256TokenHasher),
    ));

    let state = AppState {
        read,
        auth,
        content_origin: ContentOrigin::new(content_origin),
    };

    eprintln!(
        "planeter (M1 pre-tag: in-memory stores, not yet for production) listening on http://{addr}"
    );
    if let Err(e) = planeter_web::serve(state, addr).await {
        eprintln!("planeter: server error: {e}");
        std::process::exit(1);
    }
}
