//! The repository lease (RFC 004 D-7): the three classes **R**, **X**, **W**, bounded waits and a durable record of
//! who holds the exclusive ones.
//!
//! | class | what it is for | allowed alongside |
//! |---|---|---|
//! | **R** read | every read of the repository (`tree`, `log`, `verify`, the bundle export …) | R, X, W |
//! | **X** export | an `archive export`, which holds prikk's container locks for its whole run | R only |
//! | **W** write | a `sync accept`, an archive import, a seal: anything that writes the repository | R only |
//!
//! So R never waits for anything and nothing waits for R (measured on prikk 0.53.0: reads never failed during an
//! accept, ledger PK-45); X and W exclude each other and themselves. All the waiting is for **one exclusive slot per
//! repository**, and it is **bounded**: a caller names how long it will wait and gets [`LeaseError::Busy`] with a
//! retry hint when the time is up (the transport turns that into `503` with `Retry-After`).
//!
//! **Fairness.** Readers do not queue at all, so no stream of readers can starve a writer. Among those that do
//! wait for the slot, a waiting **W is served before any X** (an export that arrives while a writer is waiting
//! queues behind it), and each class is first-come-first-served. The slot goes to the head of the line when it is
//! released; a newcomer never takes a free slot ahead of someone already waiting.
//!
//! **Safety.** A [`Lease`] releases on drop, including during a panic's unwinding. A waiter that gives up (a
//! timeout, or an async caller that is cancelled while its blocking wait is running) removes itself from the line
//! and wakes the others, so a vanished waiter cannot block anyone. The table has an entry only for a repository
//! that has a holder or a waiter, so it does not grow with the number of repositories ever touched.
//!
//! **Durability.** A grant of X or W is recorded in planeter's store ([`planeter_store::LeaseRecordStore`]) and
//! the record removed on release. After a crash the records of the dead process remain, marked with that process's
//! boot id; [`LeaseManager::orphans`] lists them so that recovery (RFC 004 part 4) can see what was running.
//! **A record does not block a new grant**: whether an orphan holds a repository back until recovery has looked is
//! part 4's decision, not this one's.
//!
//! **Scope.** The lease is **in-process**: it assumes one server process per store (the deployment shape of RFC
//! 004). A second process against one store is refused at start ([`crate::process_lock`]).

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use planeter_store::{HolderRecord, LeaseRecordStore, RepoId, StoreError};

/// The three lease classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LeaseClass {
    /// **R**: a read. Never waits, never makes anyone wait.
    Read,
    /// **X**: an export. Excludes X and W.
    Export,
    /// **W**: a write. Excludes X and W.
    Write,
}

impl LeaseClass {
    /// The letter of the RFC's table.
    pub fn letter(self) -> &'static str {
        match self {
            LeaseClass::Read => "R",
            LeaseClass::Export => "X",
            LeaseClass::Write => "W",
        }
    }

    /// Whether a holder of `self` and a holder of `other` may run at the same time (the RFC's table).
    pub fn compatible_with(self, other: LeaseClass) -> bool {
        matches!(self, LeaseClass::Read) || matches!(other, LeaseClass::Read)
    }

    fn exclusive(self) -> bool {
        !matches!(self, LeaseClass::Read)
    }
}

/// Why a lease was not granted.
#[derive(Debug)]
#[non_exhaustive]
pub enum LeaseError {
    /// The wait ran out. `retry_after` is a hint for the caller (the transport sends it as `Retry-After`);
    /// `held_by` is the class of the holder at that moment, when there was one.
    Busy {
        retry_after: Duration,
        held_by: Option<LeaseClass>,
    },
    /// The holder record could not be written (the lease was not granted).
    Store(StoreError),
}

impl fmt::Display for LeaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LeaseError::Busy { retry_after, .. } => write!(
                f,
                "the repository is busy; try again in about {} s",
                retry_after.as_secs().max(1)
            ),
            LeaseError::Store(e) => write!(f, "could not record the lease holder: {e}"),
        }
    }
}

impl std::error::Error for LeaseError {}

#[derive(Debug)]
struct Holder {
    class: LeaseClass,
}

#[derive(Default)]
struct RepoState {
    readers: usize,
    holder: Option<Holder>,
    waiting_writers: VecDeque<u64>,
    waiting_exporters: VecDeque<u64>,
}

impl RepoState {
    fn is_idle(&self) -> bool {
        self.readers == 0
            && self.holder.is_none()
            && self.waiting_writers.is_empty()
            && self.waiting_exporters.is_empty()
    }

    /// May the waiter with `ticket` of `class` take the slot now?
    fn may_take(&self, class: LeaseClass, ticket: u64) -> bool {
        if self.holder.is_some() {
            return false;
        }
        match class {
            LeaseClass::Write => self.waiting_writers.front() == Some(&ticket),
            // An exporter goes only when no writer is waiting, and it is first among exporters.
            LeaseClass::Export => {
                self.waiting_writers.is_empty() && self.waiting_exporters.front() == Some(&ticket)
            }
            LeaseClass::Read => true,
        }
    }
}

#[derive(Default)]
struct Table {
    repos: HashMap<RepoId, RepoState>,
    next_ticket: u64,
    next_holder: u64,
}

struct Inner {
    table: Mutex<Table>,
    changed: Condvar,
    records: Arc<dyn LeaseRecordStore>,
    boot_id: String,
    now_unix: fn() -> u64,
    retry_after: Duration,
}

/// The lease manager of one repository store (and so of one server process). Cheap to clone.
#[derive(Clone)]
pub struct LeaseManager {
    inner: Arc<Inner>,
}

impl LeaseManager {
    /// A manager recording its exclusive grants in `records`. `boot_id` must differ between server processes
    /// (use [`new_boot_id`]); `now_unix` is the clock (tests pin it); `retry_after` is the hint a busy answer
    /// carries.
    pub fn new(
        records: Arc<dyn LeaseRecordStore>,
        boot_id: String,
        now_unix: fn() -> u64,
        retry_after: Duration,
    ) -> Self {
        LeaseManager {
            inner: Arc::new(Inner {
                table: Mutex::new(Table::default()),
                changed: Condvar::new(),
                records,
                boot_id,
                now_unix,
                retry_after,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Table> {
        // A panic never happens while the table is locked (nothing inside the lock can panic), but a poisoned
        // mutex must not take the whole forge down with it.
        self.inner
            .table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Take a lease, waiting at most `wait`. R is granted at once. The exclusive classes wait for the slot, in
    /// the order described in the module documentation, and give up with [`LeaseError::Busy`] when the time is
    /// up. **Blocking**: from async code call it inside `spawn_blocking`; a caller that is cancelled while the
    /// blocking call runs finds the lease released when the result is dropped.
    pub fn acquire(
        &self,
        repo: &RepoId,
        class: LeaseClass,
        operation: &str,
        wait: Duration,
    ) -> Result<Lease, LeaseError> {
        let deadline = Instant::now() + wait;
        let mut table = self.lock();
        if class == LeaseClass::Read {
            table.repos.entry(repo.clone()).or_default().readers += 1;
            return Ok(self.lease(repo, class, None));
        }
        // Join the line for the slot.
        let ticket = table.next_ticket;
        table.next_ticket += 1;
        {
            let st = table.repos.entry(repo.clone()).or_default();
            match class {
                LeaseClass::Write => st.waiting_writers.push_back(ticket),
                _ => st.waiting_exporters.push_back(ticket),
            }
        }
        loop {
            let st = table
                .repos
                .get(repo)
                .expect("a waiter keeps its repository's entry alive");
            if st.may_take(class, ticket) {
                break;
            }
            let now = Instant::now();
            if now >= deadline {
                let held_by = st.holder.as_ref().map(|h| h.class);
                self.leave_line(&mut table, repo, class, ticket);
                drop(table);
                self.inner.changed.notify_all();
                return Err(LeaseError::Busy {
                    retry_after: self.inner.retry_after,
                    held_by,
                });
            }
            table = self
                .inner
                .changed
                .wait_timeout(table, deadline - now)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        // Our turn: take the slot, then leave the line (in this order, so the entry is never seen idle).
        let holder_id = format!("{}-{}", self.inner.boot_id, table.next_holder);
        table.next_holder += 1;
        table.repos.get_mut(repo).expect("entry exists").holder = Some(Holder { class });
        self.leave_line(&mut table, repo, class, ticket);
        drop(table);
        // Record the grant durably. If that fails the slot is given back and the caller is told.
        let record = HolderRecord {
            repo_id: repo.clone(),
            class: class.letter().to_owned(),
            operation: operation.to_owned(),
            holder_id: holder_id.clone(),
            boot_id: self.inner.boot_id.clone(),
            since_unix: (self.inner.now_unix)(),
        };
        if let Err(e) = self.inner.records.record(&record) {
            self.release(repo, class, Some(&holder_id), false);
            return Err(LeaseError::Store(e));
        }
        Ok(self.lease(repo, class, Some(holder_id)))
    }

    /// Take a lease only if it is free right now.
    pub fn try_acquire(
        &self,
        repo: &RepoId,
        class: LeaseClass,
        operation: &str,
    ) -> Result<Lease, LeaseError> {
        self.acquire(repo, class, operation, Duration::ZERO)
    }

    fn leave_line(&self, table: &mut Table, repo: &RepoId, class: LeaseClass, ticket: u64) {
        if let Some(st) = table.repos.get_mut(repo) {
            let line = match class {
                LeaseClass::Write => &mut st.waiting_writers,
                _ => &mut st.waiting_exporters,
            };
            line.retain(|t| *t != ticket);
            if st.is_idle() {
                table.repos.remove(repo);
            }
        }
    }

    fn lease(&self, repo: &RepoId, class: LeaseClass, holder_id: Option<String>) -> Lease {
        Lease {
            manager: self.clone(),
            repo: repo.clone(),
            class,
            holder_id,
        }
    }

    fn release(
        &self,
        repo: &RepoId,
        class: LeaseClass,
        holder_id: Option<&str>,
        clear_record: bool,
    ) {
        {
            let mut table = self.lock();
            if let Some(st) = table.repos.get_mut(repo) {
                if class.exclusive() {
                    st.holder = None;
                } else {
                    st.readers = st.readers.saturating_sub(1);
                }
                if st.is_idle() {
                    table.repos.remove(repo);
                }
            }
        }
        if clear_record && let Some(id) = holder_id {
            // A record that cannot be removed shows up as an orphan after the next restart: conservative,
            // and the release itself must not fail.
            let _ = self.inner.records.clear(id);
        }
        self.inner.changed.notify_all();
    }

    /// The records of holders that this process did not grant: the exclusive leases of a process that died
    /// (RFC 004 part 4 reads these). Records of the current process are not listed.
    pub fn orphans(&self) -> Result<Vec<HolderRecord>, StoreError> {
        Ok(self
            .inner
            .records
            .list()?
            .into_iter()
            .filter(|r| r.boot_id != self.inner.boot_id)
            .collect())
    }

    /// The boot id of this process: a holder record with another one was written by a process that is not this one.
    pub fn boot_id(&self) -> &str {
        &self.inner.boot_id
    }

    /// Every holder record of `repo`, oldest first, each with whether **this process** granted it. A record of
    /// another boot belongs to a process that is gone (the server runs once per database, RFC 004 D-7, so no other
    /// can be alive); a record of this boot is a lease held now.
    pub fn records_of(&self, repo: &RepoId) -> Result<Vec<(HolderRecord, bool)>, StoreError> {
        Ok(self
            .inner
            .records
            .list()?
            .into_iter()
            .filter(|r| &r.repo_id == repo)
            .map(|r| {
                let ours = r.boot_id == self.inner.boot_id;
                (r, ours)
            })
            .collect())
    }

    /// Remove the records **this process** granted and has not yet released: a clean shutdown leaves none behind
    /// for the next start to report as a crashed holder. Returns how many were removed.
    pub fn clear_own_records(&self) -> Result<usize, StoreError> {
        let mine: Vec<HolderRecord> = self
            .inner
            .records
            .list()?
            .into_iter()
            .filter(|r| r.boot_id == self.inner.boot_id)
            .collect();
        for r in &mine {
            self.inner.records.clear(&r.holder_id)?;
        }
        Ok(mine.len())
    }

    /// Remove an orphan's record once recovery has dealt with it.
    pub fn clear_orphan(&self, holder_id: &str) -> Result<(), StoreError> {
        self.inner.records.clear(holder_id)
    }

    /// How many repositories the table has an entry for (a holder or a waiter): zero when idle. For tests and
    /// metrics.
    pub fn table_len(&self) -> usize {
        self.lock().repos.len()
    }

    /// The exclusive holder of `repo` right now, for diagnostics.
    pub fn holder_of(&self, repo: &RepoId) -> Option<LeaseClass> {
        self.lock()
            .repos
            .get(repo)
            .and_then(|s| s.holder.as_ref().map(|h| h.class))
    }
}

/// A granted lease. Dropping it releases it, on every path including a panic.
#[must_use = "a lease is released when it is dropped"]
pub struct Lease {
    manager: LeaseManager,
    repo: RepoId,
    class: LeaseClass,
    holder_id: Option<String>,
}

impl Lease {
    pub fn class(&self) -> LeaseClass {
        self.class
    }
    /// The id of this grant's durable record (`None` for a read, which leaves none).
    pub fn holder_id(&self) -> Option<&str> {
        self.holder_id.as_deref()
    }

    pub fn repo(&self) -> &RepoId {
        &self.repo
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.manager
            .release(&self.repo, self.class, self.holder_id.as_deref(), true);
    }
}

impl fmt::Debug for Lease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Lease({} on {})", self.class.letter(), self.repo)
    }
}

/// A fresh boot id for a server process: random, so a restarted process is never mistaken for its predecessor.
pub fn new_boot_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    // No `rand` dependency: the clock's nanoseconds, the process id and the address of a fresh allocation are
    // enough to make two boots differ; it is an identifier, not a secret.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let marker = Box::new(0u8);
    format!(
        "{:x}-{:x}-{:x}",
        nanos,
        std::process::id(),
        std::ptr::from_ref(&*marker) as usize
    )
}
