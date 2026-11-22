//! What the server and the operator's command (`planeter operator`, RFC 013 D-2) read **the same way**: where the database and the
//! repositories are, the file the single-process lock lives in, the repository lease manager. Shared, not copied: the server's behaviour is
//! exactly what it was, and the command cannot read the settings differently.

use std::path::PathBuf;
use std::sync::Arc;

/// The settings both need.
pub struct Paths {
    /// `PLANETER_DB`: the SQLite database file (default `./planeter.db`).
    pub db_path: String,
    /// `PLANETER_REPOS_ROOT`: where hosted repositories live (default `./planeter-repos`).
    pub repos_root: String,
}

/// Read the two paths from the environment (the server's defaults).
pub fn paths_from_env() -> Paths {
    Paths {
        db_path: std::env::var("PLANETER_DB").unwrap_or_else(|_| "./planeter.db".to_owned()),
        repos_root: std::env::var("PLANETER_REPOS_ROOT")
            .unwrap_or_else(|_| "./planeter-repos".to_owned()),
    }
}

impl Paths {
    /// The advisory lock file next to the database (RFC 004 D-7): one server process per store, and no command beside it.
    pub fn lock_file(&self) -> String {
        format!("{}.lock", self.db_path)
    }

    pub fn repos_root_path(&self) -> PathBuf {
        PathBuf::from(&self.repos_root)
    }
}

/// Unix seconds now.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The repository lease manager over the durable holder records of `db` (the server's own construction: a 5 s default wait).
pub fn lease_manager(
    records: planeter_store::SqliteLeaseRecords,
) -> Arc<planeter_core::LeaseManager> {
    Arc::new(planeter_core::LeaseManager::new(
        Arc::new(records),
        planeter_core::new_boot_id(),
        unix_now,
        std::time::Duration::from_secs(5),
    ))
}
