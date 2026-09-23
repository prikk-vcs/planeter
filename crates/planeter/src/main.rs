#![forbid(unsafe_code)]
//! `planeter` — the forge server binary.
//!
//! **M1.** The read/host spine (RFC 001–003) as a runnable server: create/host repos over sandboxed
//! prikk, browse them through the authorized, honest read API, with production **Argon2id** /
//! constant-time token hashing and a persistent **SQLite** database for every store (both landed
//! 2026-09-23). Configuration is by environment variable:
//!
//! - `PLANETER_ADDR` — bind address (default `127.0.0.1:8080`)
//! - `PLANETER_REPOS_ROOT` — where hosted repositories live (default `./planeter-repos`)
//! - `PLANETER_CONTENT_ORIGIN` — the isolated origin for raw bytes (default `http://127.0.0.1:8080`)
//! - `PLANETER_DB` — the SQLite database file (default `./planeter.db`; created if absent)

use std::sync::Arc;

use planeter_auth::{
    Argon2idHasher, Authenticator, Sha256TokenHasher, SqliteAccountStore, SqliteCredentialStore,
};
use planeter_core::{HostingService, NullAuditSink, ReadService};
use planeter_store::{
    MembershipStore, RepositoryStore, SqliteDb, SqliteMembershipStore, SqliteRepositoryStore,
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
    let db_path = std::env::var("PLANETER_DB").unwrap_or_else(|_| "./planeter.db".to_owned());

    // One SQLite database for every store; production hashers.
    let db = match SqliteDb::open(&db_path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("planeter: cannot open database {db_path}: {e}");
            std::process::exit(1);
        }
    };
    let repo_store: Arc<dyn RepositoryStore> = Arc::new(SqliteRepositoryStore::new(db.clone()));
    let hosting = Arc::new(HostingService::new(repos_root, repo_store));
    let membership: Arc<dyn MembershipStore> = Arc::new(SqliteMembershipStore::new(db.clone()));
    let read = Arc::new(ReadService::new(
        hosting,
        membership,
        Arc::new(NullAuditSink),
    ));

    let (accounts, credentials) = match (
        SqliteAccountStore::new(db.clone()),
        SqliteCredentialStore::new(db),
    ) {
        (Ok(a), Ok(c)) => (a, c),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("planeter: cannot initialise auth tables: {e}");
            std::process::exit(1);
        }
    };
    let auth = Arc::new(Authenticator::new(
        Arc::new(accounts),
        Arc::new(credentials),
        Arc::new(Argon2idHasher),
        Arc::new(Sha256TokenHasher),
    ));

    let state = AppState {
        read,
        auth,
        content_origin: ContentOrigin::new(content_origin),
    };

    eprintln!("planeter listening on http://{addr} (db: {db_path})");
    if let Err(e) = planeter_web::serve(state, addr).await {
        eprintln!("planeter: server error: {e}");
        std::process::exit(1);
    }
}
