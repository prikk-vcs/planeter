//! Advertise and fetch, the read side of the transport (RFC 004 D-2, D-7, D-11, D-12).
//!
//! A fetch of a repository's state is a **whole artifact** — a `bundle` of one published ref or an `archive` of the
//! repository — that is **exported once, verified, spooled and cached**, and then served from the spool file:
//!
//! 1. the state **fingerprint** ([`crate::fingerprint`]) is computed; with the kind and the ref it is the **cache
//!    key** and (hashed) the **`ETag`**, so a client whose copy is current gets a `304` without any export;
//! 2. a **hit** is an index entry whose spool object is still intact (re-hashed unless unchanged since it was last
//!    checked: a corrupted spool file is never served);
//! 3. a **miss** goes through **single flight** (concurrent requests for one key share one export, and its failure),
//!    a bounded number of concurrent exports per host, and the **X lease** (bounded wait: a busy repository is a
//!    typed refusal with a retry hint). Under X, prikk exports **into the spool-out mount** with its own deadline
//!    and data ceiling; X is released; then the file is **verified** (`archive verify` / `bundle verify`): an artifact
//!    that does not verify is **never served** and never kept; a verified one is renamed into the spool;
//! 4. the caller serves the bytes **from the spool file**: prikk is never streamed to a client, so a slow client can
//!    hold no lock and no process.
//!
//! A third kind, the **have-list** (`sync have <ref>`, the `PSYNCHV1` file that tells a sender what this repository already
//! holds, RFC 004 part 8), goes through the same cache key, single flight, work directory and spool, with three differences:
//! it takes the **R lease** (it only reads), no export slot (the read gate's weighted admission bounds it, and exports and
//! have-lists never starve each other), and a **check of the file itself** (the magic and a size bound) in place of a prikk
//! verify, because prikk has no verb that verifies a have-list.

use std::collections::HashMap;
use std::fs::File;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use planeter_core::admission::{ReadVerb, ceiling_for, store_size};
use planeter_core::{CallLimits, HostingService, LeaseClass, LeaseError, LeaseManager};
use planeter_prikk::{PrikkError, PrikkRepo};
use planeter_store::{RepositoryRecord, RevisionStore};

use crate::artifacts::{Digest, Spool, StoreError};
use crate::fingerprint::{files_digest, fingerprint};
use crate::metrics::{CacheStatus, Metrics};

/// What a fetch asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// One published ref's closure (`bundle export --ref`).
    Bundle,
    /// The whole repository (`archive export`), the only way to carry a received ref (PK-60).
    Archive,
    /// The have-list of one published ref (`sync have`): what this repository holds, for a sender's incremental
    /// `sync build --have` (RFC 004 part 8). Not a `fetch` kind: it has its own route.
    Have,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Bundle => "bundle",
            Kind::Archive => "archive",
            Kind::Have => "have",
        }
    }
    /// The kinds a **fetch** names (`kind=archive|bundle`); a have-list has its own route and is never parsed here.
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "bundle" => Some(Kind::Bundle),
            "archive" => Some(Kind::Archive),
            _ => None,
        }
    }
}

/// Operator settings of the export service.
#[derive(Debug, Clone)]
pub struct ExportConfig {
    /// Exports running at once on this host (default 2).
    pub max_concurrent: usize,
    /// How long a request waits for an export slot and for the X lease before it is refused as busy.
    pub lease_wait: Duration,
    /// The wall-time bound of one prikk call of an export (the export and the verify each get it), default 600 s.
    pub deadline: Duration,
    /// The data ceiling (`RLIMIT_DATA`) of every prikk call of an export, in bytes.
    pub data_ceiling: u64,
}

impl Default for ExportConfig {
    fn default() -> Self {
        ExportConfig {
            max_concurrent: 2,
            lease_wait: Duration::from_secs(5),
            deadline: Duration::from_secs(600),
            data_ceiling: 2048 << 20,
        }
    }
}

/// Why a fetch produced no artifact. Each variant is a named refusal with a stable reason code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// No export slot or no X lease within the bounded wait (or the shared export took too long to wait for).
    Busy { retry_after: Duration },
    /// A bundle needs a `ref`.
    RefRequired,
    /// The ref is not one the repository holds.
    UnknownRef,
    /// A bundle of a received ref cannot be exported (PK-60): the archive carries it.
    BundleOfReceivedRef,
    /// This ref has no have-list: only a published **branch** has one (RFC 004 part 8). prikk refuses `sync have` for a tag
    /// ("ref namespace is reserved") and a received ref is not published.
    NoHaveList,
    /// The exported file did not verify: it was not served and not kept.
    NotVerified,
    /// prikk failed (or could not run) the export; nothing was kept.
    ExportFailed,
    /// An export or verify call hit its wall-time bound.
    Deadline,
    /// An export or verify call hit its memory ceiling.
    Ceiling,
    /// The artifact is larger than the whole spool budget.
    TooLargeForSpool,
    /// The repository's state could not be read (its files, its counter, its refs).
    Unavailable,
}

impl FetchError {
    /// The stable reason code (the API's error code and the metrics' refusal key).
    pub fn reason(&self) -> &'static str {
        match self {
            FetchError::Busy { .. } => "busy",
            FetchError::RefRequired => "ref_required",
            FetchError::UnknownRef => "unknown_ref",
            FetchError::BundleOfReceivedRef => "bundle_of_received_ref",
            FetchError::NoHaveList => "no_have_list",
            FetchError::NotVerified => "not_verified",
            FetchError::ExportFailed => "export_failed",
            FetchError::Deadline => "export_deadline",
            FetchError::Ceiling => "export_ceiling",
            FetchError::TooLargeForSpool => "spool_too_small",
            FetchError::Unavailable => "unavailable",
        }
    }
}

/// The magic that opens a have-list (`prikk sync have`).
const HAVE_MAGIC: &[u8] = b"PSYNCHV1";

/// The most a have-list may be, in bytes: 8 MiB. A have-list is 32 bytes a patch plus 60 (92 B for one patch, 32,060 B for 1,000;
/// RFC 004 part 8, T0), so this is about 260,000 patches — more than twice the 100,000-patch shape the output tables call the largest
/// legitimate (3.2 MB). A bigger file is not a have-list this server will keep or serve.
pub const HAVE_MAX_BYTES: u64 = 8 << 20;

/// Whether the file prikk wrote is a have-list this server will serve: it opens with the magic and is within the bound.
fn have_list_is_sound(file: &std::path::Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = File::open(file) else {
        return false;
    };
    let Ok(meta) = f.metadata() else {
        return false;
    };
    let mut head = [0u8; HAVE_MAGIC.len()];
    meta.len() <= HAVE_MAX_BYTES && f.read_exact(&mut head).is_ok() && head == *HAVE_MAGIC
}

/// What a ref is, for the advertisement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefClass {
    Branch,
    Tag,
    /// A ref the repository **received** (`remotes/…`): not published, carried only by an archive.
    Received,
}

/// One ref the repository holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefInfo {
    pub name: String,
    pub class: RefClass,
    pub closed: bool,
    pub ref_state_id: Option<String>,
    pub tip_block: Option<String>,
    /// The tip block's `update_seq`: the ref's RefState sequence (RFC 004 D-8), read from the same one-block `log` as the tip. `None` for
    /// a tag, and for a ref whose log could not be read.
    pub update_seq: Option<u64>,
}

/// One thing a client can fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub kind: Kind,
    pub ref_name: Option<String>,
    /// The last size an export of it had (absent until the first export; it may be stale).
    pub size: Option<u64>,
    /// The `ETag` a fetch of it would return now.
    pub etag: String,
}

/// What a repository advertises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advertisement {
    pub fingerprint: String,
    pub refs: Vec<(RefInfo, Vec<Offer>)>,
    pub archive: Offer,
}

/// A fetchable artifact in the spool.
#[derive(Debug, Clone)]
pub struct Artifact {
    pub digest: Digest,
    pub size: u64,
    pub etag: String,
    pub cache: CacheStatus,
    /// How long this request waited for an export slot and the X lease (zero on a hit).
    pub lease_wait: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    repo: String,
    kind: Kind,
    ref_name: Option<String>,
    fingerprint: String,
}

impl CacheKey {
    fn etag(&self) -> String {
        let text = format!(
            "{}\0{}\0{}\0{}",
            self.repo,
            self.kind.as_str(),
            self.ref_name.as_deref().unwrap_or(""),
            self.fingerprint
        );
        format!("\"{}\"", &planeter_auth::sha256_hex(text.as_bytes())[..32])
    }
}

#[derive(Debug, Clone)]
struct IndexEntry {
    digest: Digest,
    size: u64,
}

struct Flight {
    result: Mutex<Option<Result<Artifact, FetchError>>>,
    done: Condvar,
}

/// Clears a flight when its leader ends, even by a panic: followers never wait on a dead leader.
struct FlightGuard<'a> {
    service: &'a ExportService,
    key: CacheKey,
    flight: Arc<Flight>,
}

impl FlightGuard<'_> {
    fn finish(&self, result: Result<Artifact, FetchError>) {
        *self
            .flight
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(result);
        self.flight.done.notify_all();
    }
}

impl Drop for FlightGuard<'_> {
    fn drop(&mut self) {
        {
            let mut r = self
                .flight
                .result
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if r.is_none() {
                *r = Some(Err(FetchError::ExportFailed));
            }
        }
        self.flight.done.notify_all();
        self.service
            .flights
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.key);
    }
}

/// A repository, a kind and a ref: what the last known size is kept by.
type SizeKey = (String, Kind, Option<String>);

/// The ref listing of one repository state, built once per fingerprint.
#[derive(Debug, Clone)]
struct Listing {
    fingerprint: String,
    refs: Vec<RefInfo>,
}

/// A bounded counting semaphore over a mutex (the export slots), with a timed wait.
struct Slots {
    free: Mutex<usize>,
    changed: Condvar,
}

impl Slots {
    fn acquire(&self, wait: Duration) -> bool {
        let deadline = Instant::now() + wait;
        let mut free = self
            .free
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *free == 0 {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            free = self
                .changed
                .wait_timeout(free, deadline - now)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        *free -= 1;
        true
    }
    fn release(&self) {
        *self
            .free
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        self.changed.notify_one();
    }
}

struct SlotGuard<'a>(&'a Slots);
impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        self.0.release();
    }
}

/// The export service. Cheap to share (`Arc`); every method takes `&self` and is **blocking** (call it from
/// `spawn_blocking`).
pub struct ExportService {
    hosting: Arc<HostingService>,
    spool: Arc<Spool>,
    leases: Arc<LeaseManager>,
    revisions: Arc<dyn RevisionStore>,
    config: ExportConfig,
    metrics: Arc<Metrics>,
    index: Mutex<HashMap<CacheKey, IndexEntry>>,
    /// The last size of an export, by (repo, kind, ref): for the advertisement.
    sizes: Mutex<HashMap<SizeKey, u64>>,
    flights: Mutex<HashMap<CacheKey, Arc<Flight>>>,
    listings: Mutex<HashMap<String, Listing>>,
    slots: Slots,
}

impl ExportService {
    pub fn new(
        hosting: Arc<HostingService>,
        spool: Arc<Spool>,
        leases: Arc<LeaseManager>,
        revisions: Arc<dyn RevisionStore>,
        config: ExportConfig,
        metrics: Arc<Metrics>,
    ) -> Self {
        let slots = Slots {
            free: Mutex::new(config.max_concurrent.max(1)),
            changed: Condvar::new(),
        };
        ExportService {
            hosting,
            spool,
            leases,
            revisions,
            config,
            metrics,
            index: Mutex::new(HashMap::new()),
            sizes: Mutex::new(HashMap::new()),
            flights: Mutex::new(HashMap::new()),
            listings: Mutex::new(HashMap::new()),
            slots,
        }
    }

    pub fn metrics(&self) -> &Arc<Metrics> {
        &self.metrics
    }

    pub fn config(&self) -> &ExportConfig {
        &self.config
    }

    // ------------------------------------------------------------------------------------ the state fingerprint

    /// The repository's current state fingerprint: the revision counter and the files cross-check.
    pub fn state(&self, record: &RepositoryRecord) -> Result<String, FetchError> {
        let revision = self.revisions.get(&record.repo_id).map_err(|e| {
            eprintln!("planeter: cannot read the revision counter: {e}");
            FetchError::Unavailable
        })?;
        let files = files_digest(&record.path.join(".prikk")).map_err(|e| {
            eprintln!("planeter: cannot fingerprint a repository's files: {e}");
            FetchError::Unavailable
        })?;
        Ok(fingerprint(revision, &files))
    }

    /// Record that a write to the repository has **completed**: bumps its revision counter, which changes the
    /// fingerprint, so the next fetch exports again. The caller must hold the repository's **W lease** (passed as
    /// proof; any other lease is a programming error and panics), and calls this before releasing it.
    pub fn write_completed(&self, lease: &planeter_core::Lease) -> Result<u64, StoreError> {
        assert_eq!(
            lease.class(),
            LeaseClass::Write,
            "the revision counter is bumped under the W lease"
        );
        let n = self
            .revisions
            .bump(lease.repo())
            .map_err(|e| StoreError::Io {
                context: "cannot bump the revision counter",
                source: std::io::Error::other(e.to_string()),
            })?;
        Ok(n)
    }

    // ------------------------------------------------------------------------------------ refs and advertise

    fn listing(&self, record: &RepositoryRecord, fp: &str) -> Result<Listing, FetchError> {
        let id = record.repo_id.as_str().to_owned();
        if let Some(l) = self
            .listings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&id)
            .filter(|l| l.fingerprint == fp)
        {
            return Ok(l.clone());
        }
        let driver = self
            .hosting
            .open_record_with(record, CallLimits::data_ceiling(self.config.data_ceiling))
            .map_err(|e| {
                eprintln!("planeter: cannot open a repository for its refs: {e}");
                FetchError::Unavailable
            })?;
        let unavailable = |what: &str, e: PrikkError| {
            eprintln!("planeter: cannot list a repository's {what}: {e}");
            FetchError::Unavailable
        };
        let branches = driver
            .branches(true)
            .map_err(|e| unavailable("branches", e))?;
        let tags = driver.tags().map_err(|e| unavailable("tags", e))?;
        // The tip of a ref is shown when `log` gives it. A log that cannot be read is "no tip" (the advertisement still lists the
        // ref) — except one that **printed more than it may** (IS-55): that is not "no tip", it is a repository this server cannot
        // describe, and the advertisement fails as any unreadable one does.
        let tip = |name: &str| -> Result<Option<(String, u64)>, FetchError> {
            match driver.log(Some(name), Some(1)) {
                Ok(l) => Ok(l.blocks.first().map(|b| (b.block_id.clone(), b.update_seq))),
                Err(e @ PrikkError::OutputTooLarge { .. }) => Err(unavailable("a log", e)),
                Err(_) => Ok(None),
            }
        };
        let mut refs = Vec::new();
        for b in &branches.branches {
            let t = tip(&b.ref_name)?;
            refs.push(RefInfo {
                name: b.ref_name.clone(),
                class: RefClass::Branch,
                closed: b.closed,
                ref_state_id: Some(b.ref_state_id.clone()),
                tip_block: t.as_ref().map(|(id, _)| id.clone()),
                update_seq: t.map(|(_, seq)| seq),
            });
        }
        for t in &tags.tags {
            refs.push(RefInfo {
                name: t.ref_name.clone(),
                class: RefClass::Tag,
                closed: false,
                ref_state_id: None,
                tip_block: Some(t.target_block_id.clone()),
                update_seq: None,
            });
        }
        for r in &branches.received {
            let t = tip(&r.ref_name)?;
            refs.push(RefInfo {
                name: r.ref_name.clone(),
                class: RefClass::Received,
                closed: false,
                ref_state_id: Some(r.ref_state_id.clone()),
                tip_block: t.as_ref().map(|(id, _)| id.clone()),
                update_seq: t.map(|(_, seq)| seq),
            });
        }
        let listing = Listing {
            fingerprint: fp.to_owned(),
            refs,
        };
        self.listings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, listing.clone());
        Ok(listing)
    }

    fn key(
        &self,
        record: &RepositoryRecord,
        kind: Kind,
        ref_name: Option<&str>,
        fp: &str,
    ) -> CacheKey {
        CacheKey {
            repo: record.repo_id.as_str().to_owned(),
            kind,
            ref_name: ref_name.map(str::to_owned),
            fingerprint: fp.to_owned(),
        }
    }

    fn last_size(&self, key: &CacheKey) -> Option<u64> {
        self.sizes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&(key.repo.clone(), key.kind, key.ref_name.clone()))
            .copied()
    }

    /// What the repository advertises now: its refs, the state fingerprint, and the fetches on offer with their
    /// last known sizes and the `ETag`s a fetch would return. No filesystem path appears.
    pub fn advertise(&self, record: &RepositoryRecord) -> Result<Advertisement, FetchError> {
        let fp = self.state(record)?;
        let listing = self.listing(record, &fp)?;
        let offer = |kind: Kind, ref_name: Option<&str>| {
            let key = self.key(record, kind, ref_name, &fp);
            Offer {
                kind,
                ref_name: ref_name.map(str::to_owned),
                size: self.last_size(&key),
                etag: key.etag(),
            }
        };
        let refs = listing
            .refs
            .iter()
            .map(|r| {
                let offers = match r.class {
                    // A published branch offers its bundle and its have-list (what a sender needs for an incremental push); a tag has
                    // no have-list (prikk: `sync have` takes a branch).
                    RefClass::Branch => vec![
                        offer(Kind::Bundle, Some(&r.name)),
                        offer(Kind::Have, Some(&r.name)),
                    ],
                    RefClass::Tag => vec![offer(Kind::Bundle, Some(&r.name))],
                    RefClass::Received => vec![],
                };
                (r.clone(), offers)
            })
            .collect();
        Ok(Advertisement {
            archive: offer(Kind::Archive, None),
            fingerprint: fp,
            refs,
        })
    }

    // ------------------------------------------------------------------------------------ fetch

    /// Validate a request and compute its `ETag` for the **current** state without exporting anything: a bundle
    /// needs a published ref the repository holds (a received ref points to the archive); a have-list needs a published
    /// **branch** (a tag and a received ref have none); an archive ignores `ref`.
    /// Returns the normalised ref and the `ETag`. A client whose `If-None-Match` equals it has the current copy.
    pub fn plan(
        &self,
        record: &RepositoryRecord,
        kind: Kind,
        ref_name: Option<&str>,
    ) -> Result<(Option<String>, String), FetchError> {
        let fp = self.state(record)?;
        let ref_name = match kind {
            Kind::Archive => None,
            Kind::Bundle | Kind::Have => {
                let name = ref_name.ok_or(FetchError::RefRequired)?;
                let listing = self.listing(record, &fp)?;
                match listing.refs.iter().find(|r| r.name == name) {
                    None => return Err(FetchError::UnknownRef),
                    Some(r) if kind == Kind::Have && r.class != RefClass::Branch => {
                        return Err(FetchError::NoHaveList);
                    }
                    Some(r) if r.class == RefClass::Received => {
                        return Err(FetchError::BundleOfReceivedRef);
                    }
                    Some(_) => Some(name.to_owned()),
                }
            }
        };
        let etag = self.key(record, kind, ref_name.as_deref(), &fp).etag();
        Ok((ref_name, etag))
    }

    fn cached(&self, key: &CacheKey) -> Option<Artifact> {
        let entry = self
            .index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .cloned()?;
        if self.spool.contains(&entry.digest) && self.spool.verify_object(&entry.digest) {
            Some(Artifact {
                digest: entry.digest,
                size: entry.size,
                etag: key.etag(),
                cache: CacheStatus::Hit,
                lease_wait: Duration::ZERO,
            })
        } else {
            // Evicted or corrupted: forget it; the caller exports again.
            self.index
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(key);
            None
        }
    }

    /// The artifact for a request **only if it is already in the spool** (the current state, the object intact): no
    /// export, no lease, no wait. The have-list route asks this first so that a hit is served without being admitted
    /// against the memory budget. `ref_name` must be the normalised one from [`ExportService::plan`].
    pub fn peek(
        &self,
        record: &RepositoryRecord,
        kind: Kind,
        ref_name: Option<&str>,
    ) -> Result<Option<Artifact>, FetchError> {
        let fp = self.state(record)?;
        Ok(self.cached(&self.key(record, kind, ref_name, &fp)))
    }

    /// The artifact for a request: from the spool if the current state was exported and the object is intact,
    /// otherwise exported, verified and spooled (single flight). `ref_name` must be the normalised one from
    /// [`ExportService::plan`].
    pub fn artifact(
        &self,
        record: &RepositoryRecord,
        kind: Kind,
        ref_name: Option<&str>,
    ) -> Result<Artifact, FetchError> {
        let fp = self.state(record)?;
        let key = self.key(record, kind, ref_name, &fp);
        if let Some(a) = self.cached(&key) {
            return Ok(a);
        }
        // Single flight.
        let (flight, leader) = {
            let mut flights = self
                .flights
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match flights.get(&key) {
                Some(f) => (f.clone(), false),
                None => {
                    let f = Arc::new(Flight {
                        result: Mutex::new(None),
                        done: Condvar::new(),
                    });
                    flights.insert(key.clone(), f.clone());
                    (f, true)
                }
            }
        };
        if leader {
            let guard = FlightGuard {
                service: self,
                key: key.clone(),
                flight,
            };
            let result = self.export(record, &key);
            guard.finish(result.clone());
            return result;
        }
        // A follower waits for the leader's result (its failure too), within a bound.
        let bound = self.config.lease_wait + self.config.deadline * 2 + Duration::from_secs(30);
        let end = Instant::now() + bound;
        let mut r = flight
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if let Some(res) = r.as_ref() {
                return res.clone().map(|mut a| {
                    a.cache = CacheStatus::Shared;
                    a
                });
            }
            let now = Instant::now();
            if now >= end {
                return Err(FetchError::Busy {
                    retry_after: self.config.lease_wait,
                });
            }
            r = flight
                .done
                .wait_timeout(r, end - now)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }

    /// The sandbox of one export: the repository and **this export's working directory** (read-write, the
    /// spool-out mount), and nothing else of the spool.
    fn sandbox_for_export(&self, work: &crate::artifacts::WorkDir) -> planeter_prikk::Sandbox {
        self.hosting
            .sandbox()
            .with_spool_out(work.path(), self.config.deadline.as_secs().max(1))
    }

    fn map_prikk(e: &PrikkError) -> FetchError {
        match e {
            PrikkError::CeilingExceeded { .. } => FetchError::Ceiling,
            // The export (or the verify of it) printed more than it may: the call was killed, nothing was served.
            PrikkError::OutputTooLarge { .. } => FetchError::ExportFailed,
            PrikkError::Command { code: Some(c), .. }
                if *c == planeter_prikk::sandbox::TIMEOUT_EXIT_CODE =>
            {
                FetchError::Deadline
            }
            _ => FetchError::ExportFailed,
        }
    }

    /// The leader's work: slot, X lease, export, release X, verify, adopt, index.
    fn export(&self, record: &RepositoryRecord, key: &CacheKey) -> Result<Artifact, FetchError> {
        let waited = Instant::now();
        // A have-list only reads: the R lease, and no export slot (the read gate's memory admission bounds it).
        let have = key.kind == Kind::Have;
        let _slot = if have {
            None
        } else {
            if !self.slots.acquire(self.config.lease_wait) {
                return Err(FetchError::Busy {
                    retry_after: self.config.lease_wait,
                });
            }
            Some(SlotGuard(&self.slots))
        };
        let operation = match key.kind {
            Kind::Archive => "archive export",
            Kind::Bundle => "bundle export",
            Kind::Have => "sync have",
        };
        let lease = match self.leases.acquire(
            &record.repo_id,
            if have {
                LeaseClass::Read
            } else {
                LeaseClass::Export
            },
            operation,
            self.config.lease_wait,
        ) {
            Ok(l) => l,
            Err(LeaseError::Busy { retry_after, .. }) => {
                return Err(FetchError::Busy { retry_after });
            }
            Err(e) => {
                eprintln!("planeter: cannot take an export lease: {e}");
                return Err(FetchError::Unavailable);
            }
        };
        let lease_wait = waited.elapsed();
        // The state can have changed while this request waited: the artifact is keyed by what X now holds.
        let fp = self.state(record)?;
        let key = CacheKey {
            fingerprint: fp,
            ..key.clone()
        };
        if let Some(mut a) = self.cached(&key) {
            drop(lease);
            a.cache = CacheStatus::Shared;
            return Ok(a);
        }
        let work = self.spool.new_work_dir().map_err(|e| {
            eprintln!("planeter: cannot create an export working directory: {e}");
            FetchError::Unavailable
        })?;
        let partial = work.file();
        let limits = if have {
            // The have-list's own row of the read cost table (RFC 011): the memory follows the ref's patches, not the export's.
            let size = store_size(&record.path).map_err(|e| {
                eprintln!("planeter: cannot size a repository for a have-list: {e}");
                FetchError::Unavailable
            })?;
            CallLimits::data_ceiling(ceiling_for(ReadVerb::SyncHave.cost(size), None))
        } else {
            CallLimits::data_ceiling(self.config.data_ceiling)
        };
        let driver = self
            .hosting
            .open_record_sandboxed(record, limits, self.sandbox_for_export(&work))
            .map_err(|e| {
                eprintln!("planeter: cannot open a repository for an export: {e}");
                FetchError::Unavailable
            })?;
        let exported = match key.kind {
            Kind::Archive => driver.archive_export(&partial),
            Kind::Bundle => driver.bundle_export(key.ref_name.as_deref().unwrap_or(""), &partial),
            Kind::Have => driver.sync_have(key.ref_name.as_deref().unwrap_or(""), &partial),
        };
        // X is released as soon as the bytes are written: verification and serving hold no lock.
        drop(lease);
        if let Err(e) = exported {
            self.spool.discard(&partial);
            eprintln!("planeter: an export failed: {e}");
            return Err(Self::map_prikk(&e));
        }
        let verified = match key.kind {
            Kind::Archive => driver.archive_verify(&partial).map(|r| r.passed()),
            Kind::Bundle => driver.bundle_verify(&partial).map(|()| true),
            // prikk has no verb that verifies a have-list: the file is checked here, for what it must be.
            Kind::Have => Ok(have_list_is_sound(&partial)),
        };
        match verified {
            Ok(true) => {}
            Ok(false) => {
                self.spool.discard(&partial);
                return Err(FetchError::NotVerified);
            }
            Err(e) => {
                self.spool.discard(&partial);
                eprintln!("planeter: an exported artifact did not verify: {e}");
                return Err(match Self::map_prikk(&e) {
                    FetchError::ExportFailed => FetchError::NotVerified,
                    other => other,
                });
            }
        }
        let (digest, size) = self.spool.adopt(&partial).map_err(|e| match e {
            StoreError::TooLarge { .. } => FetchError::TooLargeForSpool,
            other => {
                eprintln!("planeter: cannot spool an export: {other}");
                FetchError::Unavailable
            }
        })?;
        if !have {
            self.metrics.export(&key.repo);
        }
        {
            let mut index = self
                .index
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // One entry per (repo, kind, ref): a newer state replaces the older.
            index.retain(|k, _| {
                !(k.repo == key.repo && k.kind == key.kind && k.ref_name == key.ref_name)
            });
            index.insert(
                key.clone(),
                IndexEntry {
                    digest: digest.clone(),
                    size,
                },
            );
        }
        self.sizes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert((key.repo.clone(), key.kind, key.ref_name.clone()), size);
        Ok(Artifact {
            digest,
            size,
            etag: key.etag(),
            cache: CacheStatus::Miss,
            lease_wait,
        })
    }

    /// Open a spooled artifact for reading. The returned file stays readable even if the spool evicts the object
    /// afterwards; it holds no lease.
    pub fn open(&self, digest: &Digest) -> Option<File> {
        self.spool.get(digest)
    }

    /// The spool (for the tests that corrupt an object, and for the operator log).
    pub fn spool(&self) -> &Arc<Spool> {
        &self.spool
    }
}
