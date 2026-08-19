//! Metrics: system sampling (CPU/RAM/disk/network via sysinfo), per-client
//! rate/latency tracking and the adaptive document-limit algorithm.
//!
//! The adaptive formula (recomputed every `rate_limit.tick_seconds`):
//!
//! ```text
//! lat_err   = max(0, (p50_ms - target_latency_ms) / target_latency_ms)
//! pressure  = max(0, (cpu_pct-60)/40, (mem_pct-70)/30)      // 0..1
//! shrink    = 1 / (1 + latency_sensitivity*lat_err + pressure_sensitivity*pressure)
//! limit_new = internal * (growth_rate if shrink>=1 else shrink)
//! enforced  = clamp(round(internal * multiplier * weight), min, max)
//! ```
//!
//! When the system is calm limits grow slowly; under load they shrink fast.
//! Every input (target latency, sensitivities, growth rate, min/max,
//! multiplier, per-app weights) is editable from the dashboard.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use sysinfo::{Disks, Networks, System};

use crate::state::{AppState, LimitState, SystemSnapshot, now_ms};

pub fn median(v: &mut Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

// ---------------------------------------------------------------------------
// Container-aware sampling (Docker/K8s cgroups)
// ---------------------------------------------------------------------------
//
// In a container, /proc/meminfo, /proc/stat and /proc/mounts expose the HOST's
// view, so host-wide metrics mislead: a 0.5g container on a 16G host shows
// host RAM, and a `cpus: "1.0"` container at full tilt shows 25% on a 4-core
// host. The disk mount list double-counts the same host filesystem once per
// bind mount (/, /app, /etc/hosts-style mounts each report the full fs).
//
// All container limits are read from the cgroup at runtime — nothing is
// hardcoded. Every helper falls back to sysinfo's host-wide values when there
// is no cgroup limit (bare metal, Windows) so behavior elsewhere is unchanged.

/// Longest-prefix mount match: a specific bind mount (e.g. `/app`) wins over
/// the container root (`/`). Component-aware: `/ap` must NOT claim `/app/data`.
fn best_mount(mounts: &[String], target: &str) -> Option<usize> {
    mounts
        .iter()
        .enumerate()
        .filter(|(_, m)| mount_matches(target, m))
        .max_by_key(|(_, m)| m.len())
        .map(|(i, _)| i)
}

/// Does `mount` (normalized) contain `target`? Root (`/`, matches everything)
/// and exact-or-child matches only — no partial path components.
fn mount_matches(target: &str, mount: &str) -> bool {
    if mount == "/" {
        return true;
    }
    let m = mount.trim_end_matches('/');
    target == m || target.starts_with(&format!("{m}/"))
}

/// Absolute, forward-slash, lowercased path — canonicalized when it exists so
/// prefix comparison works against absolute mount points. The `\\?\` prefix
/// (Windows canonicalize) is stripped; on non-existent paths we fall back to
/// cwd-relative resolution.
fn normalize_path(p: &str) -> String {
    let abs = match std::fs::canonicalize(p) {
        Ok(c) => c.to_string_lossy().to_string(),
        Err(_) => match std::env::current_dir() {
            Ok(d) => d.join(p).to_string_lossy().to_string(),
            Err(_) => p.to_string(),
        },
    };
    let abs = abs.strip_prefix(r"\\?\").unwrap_or(&abs);
    normalize_mount(Path::new(abs))
}

/// Lowercased, forward-slash form of a mount point for cross-platform prefix
/// comparison (matches Windows `C:\` mounts against `C:/...` targets).
fn normalize_mount(p: &Path) -> String {
    p.to_string_lossy().to_lowercase().replace('\\', "/")
}

/// The disk whose mount point is the longest prefix of `target` — a single
/// real filesystem instead of the sum over every mount (which double-counts
// bind mounts of the same host filesystem inside a container).
fn pick_disk<'a>(disks: &'a Disks, target: &str) -> Option<&'a sysinfo::Disk> {
    let list = disks.list();
    let mounts: Vec<String> = list
        .iter()
        .map(|d| normalize_mount(d.mount_point()))
        .collect();
    best_mount(&mounts, target).map(|i| &list[i])
}

// --- CPU -------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn cgroup_cpu_pct(prev: &mut Option<(Instant, u64)>) -> Option<f64> {
    let (cores, usage_usec) = cgroup_cpu_read()?;
    if !(cores.is_finite() && cores > 0.0) {
        *prev = None;
        return None;
    }
    let now = Instant::now();
    // usage delta over wall-clock elapsed, divided by the quota's effective
    // cores: a container pinned at its 1-core quota reports 100%, not 25%.
    let pct = match *prev {
        Some((t0, u0)) => {
            let elapsed_us = now.duration_since(t0).as_micros() as f64;
            if elapsed_us > 0.0 {
                Some(
                    (usage_usec.saturating_sub(u0) as f64 / (elapsed_us * cores) * 100.0)
                        .clamp(0.0, 100.0),
                )
            } else {
                None
            }
        }
        // first sample: no delta yet — seed the baseline and fall back once
        None => None,
    };
    *prev = Some((now, usage_usec));
    pct
}

#[cfg(not(target_os = "linux"))]
fn cgroup_cpu_pct(_prev: &mut Option<(Instant, u64)>) -> Option<f64> {
    None
}

/// (effective cores from the CPU quota, cumulative usage in µs) for the
/// current cgroup. None when no quota is set (bare metal, "max") or the
/// cgroup files are unreadable → caller falls back to /proc/stat.
#[cfg(target_os = "linux")]
fn cgroup_cpu_read() -> Option<(f64, u64)> {
    cgroup_cpu_v2().or_else(cgroup_cpu_v1)
}

#[cfg(target_os = "linux")]
fn cgroup_cpu_v2() -> Option<(f64, u64)> {
    // cpu.max: "<quota> <period>"; quota "max" = unlimited
    let raw = std::fs::read_to_string("/sys/fs/cgroup/cpu.max").ok()?;
    let mut it = raw.split_whitespace();
    let quota = it.next()?;
    if quota == "max" {
        return None;
    }
    let period: f64 = it.next()?.parse().ok()?;
    let usage = std::fs::read_to_string("/sys/fs/cgroup/cpu.stat")
        .ok()?
        .lines()
        .find(|l| l.starts_with("usage_usec"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    Some((quota.parse::<f64>().ok()? / period, usage))
}

#[cfg(target_os = "linux")]
fn cgroup_cpu_v1() -> Option<(f64, u64)> {
    let quota: f64 = std::fs::read_to_string("/sys/fs/cgroup/cpu/cpu.cfs_quota_us")
        .ok()?
        .trim()
        .parse()
        .ok()?;
    if quota <= 0.0 {
        // -1 = unlimited
        return None;
    }
    let period: f64 = std::fs::read_to_string("/sys/fs/cgroup/cpu/cpu.cfs_period_us")
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let usage_ns: u64 = std::fs::read_to_string("/sys/fs/cgroup/cpu/cpuacct.usage")
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some((quota / period, usage_ns / 1000))
}

// ---------------------------------------------------------------------------
// Background tasks
// ---------------------------------------------------------------------------

/// Periodically samples system metrics, updates client rates, recomputes the
/// adaptive limits, sweeps throttles and cursors.
pub async fn metrics_loop(state: std::sync::Arc<AppState>) {
    let mut sys = System::new();
    let mut nets = Networks::new_with_refreshed_list();
    let mut disks = Disks::new_with_refreshed_list();
    let mut prev_rx: u64 = 0;
    let mut prev_tx: u64 = 0;
    let mut prev_total_requests: u64 = 0;
    // disk metric target: single filesystem (see `pick_disk`); DISK_PATH env
    // override, default = the app's working directory (repo root = /app in
    // Docker, where server.yml/config/logs live — same host filesystem as
    // the bind-mounted Mongo data dir on the prod deployment)
    let disk_target =
        normalize_path(&crate::settings::env_str("DISK_PATH").unwrap_or_else(|| ".".into()));
    let mut disk_missing_logged = false;
    // cgroup CPU usage baseline (first tick only seeds it)
    let mut cgroup_cpu_prev: Option<(Instant, u64)> = None;

    loop {
        let tick = state
            .config
            .read()
            .map(|c| c.rate_limit.tick_seconds.max(1))
            .unwrap_or(5);
        tokio::time::sleep(Duration::from_secs(tick)).await;

        // --- system snapshot ---
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        nets.refresh(true);
        disks.refresh(true);

        // CPU: cgroup quota/usage when a limit is set (container), else
        // sysinfo's host-wide /proc/stat reading (bare metal, Windows)
        let cpu_pct =
            cgroup_cpu_pct(&mut cgroup_cpu_prev).unwrap_or_else(|| sys.global_cpu_usage() as f64);

        // RAM: cgroup limit when one is set (sysinfo reads memory.max /
        // memory.current itself); cgroup "used" includes reclaimable page
        // cache, same trade-off as `docker stats`. No limit → host values.
        let (mem_total, mem_used) = match sys.cgroup_limits() {
            Some(l) if l.total_memory > 0 && l.total_memory < sys.total_memory() => {
                (l.total_memory, l.total_memory.saturating_sub(l.free_memory))
            }
            _ => (sys.total_memory(), sys.used_memory()),
        };
        let mem_pct = if mem_total > 0 {
            100.0 * mem_used as f64 / mem_total as f64
        } else {
            0.0
        };
        // disk: the single filesystem holding the disk target (NOT a sum
        // over all mounts — inside a container that double-counts the host
        // disk once per bind mount)
        let (disk_used, disk_total) = match pick_disk(&disks, &disk_target) {
            Some(d) => (
                d.total_space().saturating_sub(d.available_space()),
                d.total_space(),
            ),
            None => (0, 0),
        };
        if disk_total == 0 && !disk_missing_logged {
            disk_missing_logged = true;
            crate::state::log_line(
                "WARN",
                &format!(
                    "[metrics] no mounted filesystem matches the disk metric target ({disk_target}) — disk metrics stay at 0"
                ),
            );
        }
        let disk_pct = if disk_total > 0 {
            100.0 * disk_used as f64 / disk_total as f64
        } else {
            0.0
        };
        let mut rx: u64 = 0;
        let mut tx: u64 = 0;
        for (_, data) in nets.iter() {
            rx += data.received();
            tx += data.transmitted();
        }
        let (net_rx_kbps, net_tx_kbps) = {
            let secs = tick.max(1) as f64;
            (
                (rx.saturating_sub(prev_rx)) as f64 / 1024.0 / secs,
                (tx.saturating_sub(prev_tx)) as f64 / 1024.0 / secs,
            )
        };
        prev_rx = rx;
        prev_tx = tx;

        {
            let mut snap = state.sys.write().unwrap();
            *snap = SystemSnapshot {
                cpu_pct,
                mem_pct,
                mem_used_mb: mem_used / 1024 / 1024,
                mem_total_mb: mem_total / 1024 / 1024,
                disk_pct,
                disk_used_mb: disk_used / 1024 / 1024,
                disk_total_mb: disk_total / 1024 / 1024,
                net_rx_kbps,
                net_tx_kbps,
                uptime_s: state.started.elapsed().as_secs(),
                ts_ms: now_ms(),
            };
        }

        // --- per-client rates + limits ---
        let (min_limit, max_limit, multiplier, target_lat, ks, kg) = {
            let c = state.config.read().unwrap();
            (
                c.rate_limit.min_limit,
                c.rate_limit.max_limit,
                c.rate_limit.multiplier,
                c.rate_limit.target_latency_ms,
                (
                    c.rate_limit.latency_sensitivity,
                    c.rate_limit.pressure_sensitivity,
                ),
                c.rate_limit.growth_rate,
            )
        };
        let pressure = ((cpu_pct - 60.0) / 40.0)
            .max((mem_pct - 70.0) / 30.0)
            .max(0.0);

        // collect all client keys — rates/history are computed for BOTH
        // app-level and name-level entries (so name sparklines/rps work);
        // adaptive limits only apply to apps
        let client_keys: Vec<String> = state.clients.iter().map(|e| e.key().to_string()).collect();

        for key in &client_keys {
            let Some(stats) = state.clients.get(key) else {
                continue;
            };
            // p50 from the ring
            let p50 = {
                let ring = stats.lat.lock().unwrap();
                let mut v: Vec<f64> = ring.iter().copied().collect();
                median(&mut v)
            };
            // rate EMA (delta of the cumulative counter since the last tick,
            // so it decays to 0 when traffic stops)
            let total_now = stats.total.load(Ordering::Relaxed);
            let delta = total_now - stats.last_total.swap(total_now, Ordering::Relaxed);
            let rate = delta as f64 / tick.max(1) as f64;
            let alpha = state
                .config
                .read()
                .map(|c| c.rate_limit.ema_alpha.clamp(0.01, 0.9))
                .unwrap_or(0.2);
            let ema = alpha * rate + (1.0 - alpha) * stats.rate_f64();
            stats.set_rate(ema);
            if let Ok(mut h) = stats.history.lock() {
                h.push_back(ema as f32);
                if h.len() > 120 {
                    h.pop_front();
                }
            }

            // adaptive limit (apps only)
            if !key.starts_with("app:") {
                continue;
            }
            let app = &key[4..];
            let lat_err = ((p50 - target_lat) / target_lat.max(1.0)).max(0.0);
            let shrink = 1.0 / (1.0 + ks.0 * lat_err + ks.1 * pressure);
            let mut entry = state
                .limits
                .entry(app.to_string())
                .or_insert_with(|| LimitState {
                    internal: max_limit as f64,
                    enforced: max_limit,
                    ..Default::default()
                });
            let internal = if shrink >= 1.0 {
                (entry.internal * kg).clamp(min_limit as f64, max_limit as f64)
            } else {
                (entry.internal * shrink).clamp(min_limit as f64, max_limit as f64)
            };
            let weight = state
                .config
                .read()
                .unwrap()
                .rate_limit
                .weights
                .get(app)
                .copied()
                .unwrap_or(1.0)
                .clamp(0.1, 10.0);
            let enforced =
                ((internal * multiplier * weight).round() as u32).clamp(min_limit, max_limit);
            entry.internal = internal;
            entry.enforced = enforced;
            entry.lat_err = lat_err;
            entry.pressure = pressure;
            entry.shrink = shrink;
            entry.p50_ms = p50;
            entry.rate = ema;
            entry.updated_ms = now_ms();
        }

        // global p50 + qps
        {
            let ring = state.latencies.lock().unwrap();
            let mut v: Vec<f64> = ring.iter().copied().collect();
            let p50 = median(&mut v);
            let total = state.total_requests.load(Ordering::Relaxed);
            let qps = (total.saturating_sub(prev_total_requests)) as f64 / tick.max(1) as f64;
            prev_total_requests = total;
            if let Ok(mut h) = state.lat_p50_hist.lock() {
                h.push_back(p50);
                if h.len() > 64 {
                    h.pop_front();
                }
            }
            *state.qps.write().unwrap() = qps;
        }

        crate::auth::throttle_sweep(&state);
        crate::dbq::cursor_sweep(&state);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn best_mount_prefers_longest_prefix() {
        let mounts = vec!["/".into(), "/app".into()];
        // /app is a longer prefix -> wins over the container root
        assert_eq!(best_mount(&mounts, "/app/src/main.rs"), Some(1));
        // target exactly at the mount point matches
        assert_eq!(best_mount(&mounts, "/app"), Some(1));
        // outside /app -> falls back to / (root matches everything)
        assert_eq!(best_mount(&mounts, "/etc/hosts"), Some(0));
        assert_eq!(best_mount(&mounts, "/mnt/x"), Some(0));
        // nothing matches when the root mount is absent
        assert_eq!(best_mount(&["/app".into()], "/mnt/x"), None);
    }

    #[test]
    fn best_mount_ignores_partial_component_matches() {
        // /ap is a string prefix of /app/data but NOT a component prefix
        assert_eq!(best_mount(&["/ap".into()], "/app/data"), None);
        // same on Windows drive mounts
        assert_eq!(best_mount(&["c:/dat".into()], "c:/database"), None);
    }

    #[test]
    fn normalize_mount_is_lowercase_forward_slash() {
        assert_eq!(normalize_mount(Path::new(r"C:\Users\X")), "c:/users/x");
        assert_eq!(normalize_mount(Path::new("/app")), "/app");
    }

    #[test]
    fn best_mount_matches_windows_drive_letters() {
        let mounts = vec!["c:/".into(), "d:/".into(), "c:/data".into()];
        assert_eq!(best_mount(&mounts, "c:/users/x/code"), Some(0));
        assert_eq!(best_mount(&mounts, "d:/data"), Some(1));
        // longer component prefix wins over the bare drive
        assert_eq!(best_mount(&mounts, "c:/data/db"), Some(2));
    }
}
