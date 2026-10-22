//! The read memory budget's size and where it came from (RFC 011 D-1, T4).
//!
//! The budget is the most prikk memory, by estimate, that reads may hold at once. The default is **one
//! quarter of the memory the process may use** — its cgroup's `memory.max` if one is set (cgroup v2;
//! `max` means none), otherwise physical memory — **floored at 256 MiB and capped at 8 GiB**. The cap
//! bounds only the *default*: **an explicit `PLANETER_PRIKK_MEMORY_BUDGET_MB` is never capped**, so a large
//! host raises it deliberately. The pure parts take their inputs as arguments so they can be tested; the
//! readers of `/proc` and `/sys/fs/cgroup` are plain file reads (no dependency).

/// One mebibyte.
pub const MIB: u64 = 1 << 20;

/// The least a default budget is (a deployment whose quarter is smaller still serves reads one at a time).
pub const DEFAULT_FLOOR_BYTES: u64 = 256 * MIB;

/// The most a *default* budget is: about twice the worst single call measured with the worst-case
/// multiples (a `verify` of a 1 GiB repository, ≈ 4.1 GiB). Decided by the owner, 2026-10-18.
pub const DEFAULT_CAP_BYTES: u64 = 8 * 1024 * MIB;

/// The variable that sets the budget, in MiB.
pub const VAR: &str = "PLANETER_PRIKK_MEMORY_BUDGET_MB";

/// Where the budget's value came from (stated in the start-up log).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetSource {
    /// A quarter of the usable memory.
    Default,
    /// A quarter of the usable memory, raised to the 256 MiB floor.
    DefaultFloored,
    /// A quarter of the usable memory, lowered to the 8 GiB cap.
    DefaultCapped,
    /// `PLANETER_PRIKK_MEMORY_BUDGET_MB` (never capped).
    Variable,
}

impl BudgetSource {
    /// The label used in the start-up log line.
    pub fn label(self) -> &'static str {
        match self {
            BudgetSource::Default => "default",
            BudgetSource::DefaultFloored => "default, floored at 256 MiB",
            BudgetSource::DefaultCapped => "default, capped at 8 GiB",
            BudgetSource::Variable => VAR,
        }
    }
}

/// The default budget for `usable_bytes` of memory: a quarter, floored and capped.
pub fn default_budget(usable_bytes: u64) -> (u64, BudgetSource) {
    let quarter = usable_bytes / 4;
    if quarter < DEFAULT_FLOOR_BYTES {
        (DEFAULT_FLOOR_BYTES, BudgetSource::DefaultFloored)
    } else if quarter > DEFAULT_CAP_BYTES {
        (DEFAULT_CAP_BYTES, BudgetSource::DefaultCapped)
    } else {
        (quarter, BudgetSource::Default)
    }
}

/// Whether the heavy pool admits an accept of the given admission `cost` (RFC 004 D-5.3): a budget of 0 means no gate (everything
/// is admitted); otherwise the cost must fit the heavy pool, or the proposal is refused as too large before anything is accepted.
/// The start-up log says which of the two it is.
pub fn accept_admitted(cost_bytes: u64, heavy_pool_bytes: u64, budget_bytes: u64) -> bool {
    budget_bytes == 0 || cost_bytes <= heavy_pool_bytes
}

/// The budget from the variable's value (`None`/empty = unset → the default for `usable_bytes`). An
/// explicit value is a positive integer number of MiB and is **never capped**.
pub fn budget_from_var(
    value: Option<&str>,
    usable_bytes: u64,
) -> Result<(u64, BudgetSource), String> {
    match value {
        None | Some("") => Ok(default_budget(usable_bytes)),
        Some(s) => match s.trim().parse::<u64>() {
            Ok(mb) if mb >= 1 => mb
                .checked_mul(MIB)
                .map(|bytes| (bytes, BudgetSource::Variable))
                .ok_or_else(|| format!("{VAR} is too large")),
            _ => Err(format!("{VAR} must be a positive integer (MiB), got {s:?}")),
        },
    }
}

/// The memory the process may use, from the text of its cgroup's `memory.max` (if it has one) and of
/// `/proc/meminfo`: the cgroup limit when it is a number, otherwise `MemTotal`.
pub fn usable_memory(cgroup_memory_max: Option<&str>, meminfo: &str) -> Option<u64> {
    if let Some(limit) = cgroup_memory_max
        .map(str::trim)
        .and_then(|t| t.parse::<u64>().ok())
    {
        return Some(limit);
    }
    meminfo.lines().find_map(|l| {
        let rest = l.strip_prefix("MemTotal:")?;
        let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
        kb.checked_mul(1024)
    })
}

/// Read the process's usable memory from the running system (cgroup v2 `memory.max` of its own cgroup,
/// else `MemTotal`). `Err` only when neither source is readable.
pub fn read_usable_memory() -> Result<u64, String> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let cgroup_max = std::fs::read_to_string("/proc/self/cgroup")
        .ok()
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("0::").map(str::to_owned))
        })
        .and_then(|path| {
            std::fs::read_to_string(format!(
                "/sys/fs/cgroup{}/memory.max",
                path.trim_end_matches('/')
            ))
            .ok()
        });
    usable_memory(cgroup_max.as_deref(), &meminfo)
        .ok_or_else(|| "cannot determine the memory this process may use".to_owned())
}
