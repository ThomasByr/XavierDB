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

use std::sync::atomic::Ordering;
use std::time::Duration;

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

        let cpu_pct = sys.global_cpu_usage() as f64;
        let mem_total = sys.total_memory();
        let mem_used = sys.used_memory();
        let mem_pct = if mem_total > 0 {
            100.0 * mem_used as f64 / mem_total as f64
        } else {
            0.0
        };
        let (disk_used, disk_total) = disks.iter().fold((0u64, 0u64), |(u, t), d| {
            (
                u + d.total_space().saturating_sub(d.available_space()),
                t + d.total_space(),
            )
        });
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
