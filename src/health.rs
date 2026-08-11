//! /health: a small, cached, unauthenticated status document.
//!
//! The document is refreshed in the background every
//! `health.cache_ttl_seconds` (default 5s) so that even a spammed /health
//! never touches MongoDB itself — it only reads the cached snapshot.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::Serialize;

use crate::metrics::median;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct HealthDoc {
    pub status: String, // "ok" | "degraded" | "unhealthy"
    pub checked_at_ms: i64,
    pub next_refresh_seconds: u64,
    pub compute_latency_ms: f64,
    pub qps: f64,
    pub app: AppHealth,
    pub mongodb: MongoHealth,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppHealth {
    pub status: String,
    pub uptime_s: u64,
    pub p50_latency_ms: f64,
    pub total_requests: u64,
    pub active_cursors: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct MongoHealth {
    pub reachable: bool,
    pub ping_latency_ms: f64,
    pub error: Option<String>,
}

pub async fn refresh_health(state: &Arc<AppState>) -> HealthDoc {
    // MongoDB ping with a hard timeout
    let ping_start = std::time::Instant::now();
    let mongo = tokio::time::timeout(
        Duration::from_secs(2),
        state
            .mongo
            .database("admin")
            .run_command(bson::doc! { "ping": 1 }),
    )
    .await;

    let (reachable, ping_latency_ms, error) = match mongo {
        Ok(Ok(_)) => (true, ping_start.elapsed().as_secs_f64() * 1000.0, None),
        Ok(Err(e)) => (false, 0.0, Some(crate::error::sanitize(&e.to_string()))),
        Err(_) => (false, 0.0, Some("ping timed out".to_string())),
    };

    let (p50, _) = {
        let ring = state.latencies.lock().unwrap();
        let mut v: Vec<f64> = ring.iter().copied().collect();
        (median(&mut v), ())
    };
    let qps = *state.qps.read().unwrap();
    let total = state.total_requests.load(Ordering::Relaxed);
    let cursors = state.cursors.len();

    let cpu = state.sys.read().unwrap().cpu_pct;

    let status = if !reachable {
        "unhealthy"
    } else if ping_latency_ms > 500.0 || p50 > 500.0 || cpu > 90.0 {
        "degraded"
    } else {
        "ok"
    };

    let ttl = state
        .config
        .read()
        .map(|c| c.health.cache_ttl_seconds.max(1))
        .unwrap_or(5);

    HealthDoc {
        status: status.to_string(),
        checked_at_ms: crate::state::now_ms(),
        next_refresh_seconds: ttl,
        compute_latency_ms: p50,
        qps,
        app: AppHealth {
            status: status.to_string(),
            uptime_s: state.started.elapsed().as_secs(),
            p50_latency_ms: p50,
            total_requests: total,
            active_cursors: cursors,
        },
        mongodb: MongoHealth {
            reachable,
            ping_latency_ms,
            error,
        },
    }
}

/// Background loop that keeps the cached health document fresh.
pub async fn health_loop(state: Arc<AppState>) {
    loop {
        let ttl = state
            .config
            .read()
            .map(|c| c.health.cache_ttl_seconds.max(1))
            .unwrap_or(5);
        let doc = refresh_health(&state).await;
        *state.health_cache.write().unwrap() =
            Some(serde_json::to_value(doc).unwrap_or(serde_json::Value::Null));
        tokio::time::sleep(Duration::from_secs(ttl)).await;
    }
}
