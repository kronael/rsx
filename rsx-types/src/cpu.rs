//! Concentrated hot-thread CPU setup — ONE place, reused by every pinned tile
//! (risk, ME, gateway egress, marketdata, mark). See `notes/hot-path.md` for
//! the rules this enforces: pin to an isolated core, lock + pre-fault memory,
//! never page-fault on the hot path.
//!
//! Best-effort and non-fatal: a tile must still run if pinning/mlock is denied
//! (no `CAP_IPC_LOCK`, a non-isolated dev box, etc.). `setup_hot_thread`
//! reports what it managed to do; the CALLER logs it (so rsx-types needs no
//! logging dependency).

use std::fmt;

/// What `setup_hot_thread` achieved for the current thread. Log at startup.
#[derive(Clone, Copy, Debug)]
pub struct HotSetup {
    pub core: usize,
    /// Thread pinned to `core`.
    pub pinned: bool,
    /// Process address space locked + pre-faulted (`mlockall`).
    pub mlocked: bool,
    /// `core` is in the kernel's isolated set (isolcpus/nohz_full).
    /// `None` if it could not be determined.
    pub isolated: Option<bool>,
}

impl fmt::Display for HotSetup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let iso = match self.isolated {
            Some(true) => "yes",
            Some(false) => "NO (not in isolcpus — expect tail spikes)",
            None => "unknown",
        };
        write!(
            f,
            "hot-thread core={} pinned={} mlocked={} isolated={}",
            self.core, self.pinned, self.mlocked, iso,
        )
    }
}

/// Set up the current thread for a pinned busy-loop: pin to `core`, lock +
/// pre-fault memory (`mlockall`), and check core isolation. Best-effort;
/// returns a [`HotSetup`] the caller should log. Call ONCE at the top of the
/// tile, before the hot loop.
pub fn setup_hot_thread(core: usize) -> HotSetup {
    HotSetup {
        core,
        pinned: pin_current(core),
        mlocked: mlock_all(),
        isolated: core_is_isolated(core),
    }
}

/// Pin the current thread to logical core `core`. Returns false if the index
/// is out of range or the OS refused.
pub fn pin_current(core: usize) -> bool {
    match core_affinity::get_core_ids() {
        Some(ids) => match ids.get(core) {
            Some(id) => core_affinity::set_for_current(*id),
            None => false,
        },
        None => false,
    }
}

/// `mlockall(MCL_CURRENT | MCL_FUTURE)` — lock + pre-fault all current and
/// future pages so the hot path never page-faults. Needs `CAP_IPC_LOCK` or a
/// sufficient `RLIMIT_MEMLOCK`; returns false (non-fatal) otherwise.
pub fn mlock_all() -> bool {
    // SAFETY: mlockall takes only flags and has no memory-safety contract.
    unsafe { libc::mlockall(libc::MCL_CURRENT | libc::MCL_FUTURE) == 0 }
}

/// Is `core` in the kernel's isolated set? Reads
/// `/sys/devices/system/cpu/isolated` (the effective isolcpus list). `None`
/// if the file cannot be read.
pub fn core_is_isolated(core: usize) -> Option<bool> {
    let raw =
        std::fs::read_to_string("/sys/devices/system/cpu/isolated").ok()?;
    Some(parse_cpu_list(raw.trim()).contains(&core))
}

/// Parse a Linux CPU list like `"2-3,5,7-9"` into the set of CPU indices.
pub fn parse_cpu_list(s: &str) -> Vec<usize> {
    let mut out = Vec::new();
    if s.is_empty() {
        return out;
    }
    for part in s.split(',') {
        if let Some((a, b)) = part.split_once('-') {
            if let (Ok(a), Ok(b)) =
                (a.parse::<usize>(), b.parse::<usize>())
            {
                out.extend(a..=b);
            }
        } else if let Ok(c) = part.parse::<usize>() {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
#[path = "cpu_test.rs"]
mod cpu_test;
