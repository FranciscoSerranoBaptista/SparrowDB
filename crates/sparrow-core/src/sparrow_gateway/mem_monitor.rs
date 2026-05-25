/// Memory pressure monitor.
///
/// Provides two things:
/// 1. Point-in-time helpers (`read_rss_kb`, `read_cgroup_limit_kb`, `read_mem_total_kb`,
///    `read_effective_limit_kb`, `read_thread_count`) that diagnostics and the SIGTERM log
///    call on demand.
/// 2. A long-running Tokio task (`run_memory_monitor`) that polls every 5 s and emits
///    `WARN` when the process RSS reaches ≥ 80 % of the **effective memory limit**.
///
/// ## Effective limit vs. cgroup limit
///
/// Docker Compose sets `mem_limit` which becomes the cgroup limit.  In Docker Desktop the
/// cgroup limit is set on the *container* (e.g. 24 GB) but the underlying VM has less
/// physical RAM (e.g. 15.65 GB).  Using only the cgroup limit meant the 80% warning
/// never fired even as the Docker VM ran out of physical memory.
///
/// `read_effective_limit_kb()` returns `min(cgroup_limit, MemTotal)` — the tighter of the
/// two constraints — so the monitor and the warning threshold track the real ceiling.
///
/// All file reads are Linux-only (`/proc/self/status`, `/proc/meminfo`, `/sys/fs/cgroup/…`).
/// On macOS / Windows every function returns 0 and the monitor task is a no-op.

use std::time::Duration;
use tokio::sync::watch;
use tracing::{debug, warn};

// ── point-in-time helpers ──────────────────────────────────────────────────

/// Current process RSS in kibibytes (from `/proc/self/status`).
/// Returns 0 on non-Linux or if the file cannot be read.
pub fn read_rss_kb() -> u64 {
    #[cfg(target_os = "linux")]
    {
        read_status_field("VmRSS").unwrap_or(0)
    }
    #[cfg(not(target_os = "linux"))]
    0
}

/// Number of OS threads in the current process (from `/proc/self/status`).
/// Returns 0 on non-Linux or if the file cannot be read.
pub fn read_thread_count() -> u32 {
    #[cfg(target_os = "linux")]
    {
        read_status_field("Threads").unwrap_or(0) as u32
    }
    #[cfg(not(target_os = "linux"))]
    0
}

/// Container / cgroup memory limit in kibibytes.
///
/// Tries cgroup v2 (`/sys/fs/cgroup/memory.max`) first, then cgroup v1
/// (`/sys/fs/cgroup/memory/memory.limit_in_bytes`).
/// Returns 0 if the limit is "unlimited" or cannot be read.
pub fn read_cgroup_limit_kb() -> u64 {
    #[cfg(target_os = "linux")]
    {
        // cgroup v2
        if let Ok(content) = std::fs::read_to_string("/sys/fs/cgroup/memory.max") {
            let s = content.trim();
            if s != "max" {
                if let Ok(bytes) = s.parse::<u64>() {
                    return bytes / 1024;
                }
            }
        }
        // cgroup v1
        if let Ok(content) =
            std::fs::read_to_string("/sys/fs/cgroup/memory/memory.limit_in_bytes")
        {
            if let Ok(bytes) = content.trim().parse::<u64>() {
                // v1 uses ~2^63 to signal "unlimited"
                const UNLIMITED: u64 = 0x7FFF_FFFF_FFFF_F000;
                if bytes < UNLIMITED {
                    return bytes / 1024;
                }
            }
        }
        0
    }
    #[cfg(not(target_os = "linux"))]
    0
}

/// Physical RAM available to the host / Docker VM in kibibytes (from `/proc/meminfo`).
///
/// This is the `MemTotal` field — the total amount of RAM the OS reports. In a Docker Desktop
/// environment this reflects the VM's physical memory ceiling, which may be lower than the
/// cgroup limit set by `mem_limit` in docker-compose.
///
/// Returns 0 on non-Linux or if `/proc/meminfo` cannot be read.
pub fn read_mem_total_kb() -> u64 {
    #[cfg(target_os = "linux")]
    {
        read_meminfo_field("MemTotal").unwrap_or(0)
    }
    #[cfg(not(target_os = "linux"))]
    0
}

/// The tighter of `read_cgroup_limit_kb()` and `read_mem_total_kb()`.
///
/// Use this as the memory pressure threshold rather than the raw cgroup limit, because in
/// Docker Desktop the cgroup limit (set via `mem_limit:`) can exceed the Docker VM's physical
/// RAM, making the cgroup limit an ineffective OOM predictor.
///
/// Returns `min(cgroup_limit, mem_total)` when both are non-zero.
/// Falls back to whichever single value is non-zero, or 0 if both are unavailable.
pub fn read_effective_limit_kb() -> u64 {
    #[cfg(target_os = "linux")]
    {
        let cgroup = read_cgroup_limit_kb();
        let physical = read_mem_total_kb();
        match (cgroup, physical) {
            (0, p) => p,
            (c, 0) => c,
            (c, p) => c.min(p),
        }
    }
    #[cfg(not(target_os = "linux"))]
    0
}

// ── internal helpers ───────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn read_meminfo_field(field: &str) -> Option<u64> {
    let content = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in content.lines() {
        if line.starts_with(field) {
            // Lines look like "MemTotal:   16384000 kB"
            return line.split_whitespace().nth(1)?.parse().ok();
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn read_status_field(field: &str) -> Option<u64> {
    let content = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in content.lines() {
        if line.starts_with(field) {
            // Lines look like "VmRSS:  524288 kB" or "Threads:    226"
            return line.split_whitespace().nth(1)?.parse().ok();
        }
    }
    None
}

// ── background monitor task ────────────────────────────────────────────────

/// Spawn this as a Tokio task.
///
/// Polls RSS every 5 seconds; emits `WARN` once per poll tick when RSS ≥ 80 %
/// of the **effective memory limit** (`min(cgroup_limit, MemTotal)`).  Using the
/// effective limit rather than the raw cgroup limit ensures the warning fires in
/// Docker Desktop environments where `mem_limit:` in docker-compose can exceed the
/// VM's physical RAM.
///
/// Exits cleanly when the `shutdown_rx` watch value changes (or the sender is dropped).
pub async fn run_memory_monitor(mut shutdown_rx: watch::Receiver<bool>) {
    let effective_limit_kb = read_effective_limit_kb();
    let cgroup_limit_kb = read_cgroup_limit_kb();
    let mem_total_kb = read_mem_total_kb();

    if effective_limit_kb == 0 {
        debug!("No memory limit detected — RSS pressure monitor will not emit warnings");
        // Still enter the loop so we respond to the shutdown signal cleanly.
    } else {
        let effective_mb = effective_limit_kb / 1024;
        let cgroup_mb = cgroup_limit_kb / 1024;
        let physical_mb = mem_total_kb / 1024;
        debug!(
            effective_limit_mb = effective_mb,
            cgroup_limit_mb = cgroup_mb,
            physical_ram_mb = physical_mb,
            "Memory monitor started — will warn at ≥80% of effective limit (min of cgroup and physical RAM)"
        );
    }

    let mut interval = tokio::time::interval(Duration::from_secs(5));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if effective_limit_kb == 0 {
                    continue;
                }
                let rss_kb = read_rss_kb();
                if rss_kb == 0 {
                    continue;
                }
                let pct = rss_kb as f64 / effective_limit_kb as f64 * 100.0;
                if pct >= 80.0 {
                    warn!(
                        rss_kb,
                        effective_limit_kb,
                        cgroup_limit_kb,
                        mem_total_kb,
                        rss_pct = format!("{pct:.1}"),
                        "Memory pressure: RSS is ≥80% of effective memory limit — risk of OOM kill"
                    );
                }
            }
            result = shutdown_rx.changed() => {
                // result is Err when all senders are dropped — treat both as shutdown
                let _ = result;
                debug!("Memory monitor received shutdown signal");
                break;
            }
        }
    }
}
