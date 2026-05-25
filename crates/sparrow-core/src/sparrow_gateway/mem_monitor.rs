/// Memory pressure monitor.
///
/// Provides two things:
/// 1. Point-in-time helpers (`read_rss_kb`, `read_cgroup_limit_kb`, `read_thread_count`)
///    that diagnostics and the SIGTERM log call on demand.
/// 2. A long-running Tokio task (`run_memory_monitor`) that polls every 5 s and
///    emits `WARN` when the process RSS reaches ≥ 80 % of the cgroup memory limit.
///
/// All file reads are Linux-only (`/proc/self/status`, `/sys/fs/cgroup/…`).
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

// ── internal helper ────────────────────────────────────────────────────────

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
/// of the cgroup limit.  Exits cleanly when the `shutdown_rx` watch value
/// changes (or when the sender is dropped).
pub async fn run_memory_monitor(mut shutdown_rx: watch::Receiver<bool>) {
    let limit_kb = read_cgroup_limit_kb();

    if limit_kb == 0 {
        debug!("No cgroup memory limit detected — RSS pressure monitor will not emit warnings");
        // Still enter the loop so we respond to the shutdown signal cleanly.
    } else {
        let limit_mb = limit_kb / 1024;
        debug!(limit_mb, "Memory monitor started — will warn at ≥80% of container limit");
    }

    let mut interval = tokio::time::interval(Duration::from_secs(5));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if limit_kb == 0 {
                    continue;
                }
                let rss_kb = read_rss_kb();
                if rss_kb == 0 {
                    continue;
                }
                let pct = rss_kb as f64 / limit_kb as f64 * 100.0;
                if pct >= 80.0 {
                    warn!(
                        rss_kb,
                        limit_kb,
                        rss_pct = format!("{pct:.1}"),
                        "Memory pressure: RSS is ≥80% of container limit — risk of OOM kill"
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
