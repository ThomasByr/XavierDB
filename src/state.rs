//! Central application state shared across all Tokio workers.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicI64, AtomicU64};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use dashmap::DashMap;
use mongodb::Client;

use crate::config::ConfigFile;
use crate::perms::PermissionsFile;

pub struct AppState {
    pub config: RwLock<ConfigFile>,
    pub perms: RwLock<PermissionsFile>,

    /// JWT signing secret (fixed for the process lifetime).
    pub jwt_secret: [u8; 32],
    /// True when HTTPS is active (cookie Secure flag).
    pub https: bool,

    /// Per-identity live stats. Key: "app:<app>" or "name:<name>@<app>".
    pub clients: DashMap<String, ClientStats>,
    /// Current adaptive limit per app_id.
    pub limits: DashMap<String, LimitState>,
    /// System metrics snapshot (refreshed by the metrics task).
    pub sys: RwLock<SystemSnapshot>,
    /// Global request-processing latency samples (ms), p50 computed on tick.
    pub latencies: Mutex<VecDeque<f64>>,
    /// history of global p50 samples (last 64 ticks)
    pub lat_p50_hist: Mutex<VecDeque<f64>>,
    /// global QPS over the last tick
    pub qps: RwLock<f64>,
    /// cached /health document (JSON), refreshed by the health loop
    pub health_cache: RwLock<Option<serde_json::Value>>,
    /// Total /q requests handled (for QPS).
    pub total_requests: AtomicU64,
    /// Cursors issued/seen, keyed by cursor id.
    pub cursors: DashMap<String, CursorInfo>,
    pub cursor_seq: AtomicU64,

    /// Admin dashboard sessions: token -> (username, expires_ms).
    pub sessions: DashMap<String, AdminSession>,
    /// /auth throttle per IP: ip -> (window_start_ms, count).
    pub auth_throttle: DashMap<String, (i64, u32)>,

    /// In-memory log ring buffer (last ~1500 lines).
    pub logs: Mutex<VecDeque<String>>,

    /// MongoDB client (lazy connect).
    pub mongo: Client,
    pub started: Instant,

    /// Bytes of the last config/perms content written by this process, used
    /// to ignore our own writes in the file watcher.
    pub last_config_written: Mutex<Option<Vec<u8>>>,
    pub last_perms_written: Mutex<Option<Vec<u8>>>,

    /// Bumped whenever config or perms change (dashboard change detection).
    pub cfg_version: AtomicU64,
    pub perms_version: AtomicU64,

    /// Paths (resolved at startup).
    pub config_path: std::path::PathBuf,
    pub perms_path: std::path::PathBuf,
    /// Admin username from .env.
    pub admin_user: String,
    /// Max documents per insert batch (MAX_INSERT_BATCH env, default 1000).
    pub max_insert_batch: usize,
}

impl AppState {
    /// Build a fully initialized state.
    pub fn new(
        config: ConfigFile,
        perms: PermissionsFile,
        jwt_secret: [u8; 32],
        https: bool,
        mongo: Client,
        config_path: std::path::PathBuf,
        perms_path: std::path::PathBuf,
        admin_user: String,
        max_insert_batch: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            config: RwLock::new(config),
            perms: RwLock::new(perms),
            jwt_secret,
            https,
            clients: DashMap::new(),
            limits: DashMap::new(),
            sys: RwLock::new(SystemSnapshot::default()),
            latencies: Mutex::new(VecDeque::with_capacity(2048)),
            lat_p50_hist: Mutex::new(VecDeque::with_capacity(64)),
            qps: RwLock::new(0.0),
            health_cache: RwLock::new(None),
            total_requests: AtomicU64::new(0),
            cursors: DashMap::new(),
            cursor_seq: AtomicU64::new(0),
            sessions: DashMap::new(),
            auth_throttle: DashMap::new(),
            logs: Mutex::new(VecDeque::new()),
            mongo,
            started: Instant::now(),
            last_config_written: Mutex::new(None),
            last_perms_written: Mutex::new(None),
            cfg_version: AtomicU64::new(0),
            perms_version: AtomicU64::new(0),
            config_path,
            perms_path,
            admin_user,
            max_insert_batch,
        })
    }
}

// ---------------------------------------------------------------------------
// Per-client stats
// ---------------------------------------------------------------------------

pub struct ClientStats {
    #[allow(dead_code)]
    pub name: String,
    #[allow(dead_code)]
    pub app: String,
    /// total requests (atomic bump per request)
    pub total: AtomicU64,
    /// value of `total` at the previous metrics tick (for delta-based rate)
    pub last_total: AtomicU64,
    /// last request time (epoch ms)
    pub last_seen: AtomicI64,
    /// EMA of requests/second (f64 stored as bits), updated by the metrics task
    pub rate: AtomicU64,
    /// last latency samples (ms) for p50, pushed per request
    pub lat: Mutex<VecDeque<f64>>,
    /// smoothed rate history for sparklines (updated by the metrics task)
    pub history: Mutex<VecDeque<f32>>,
}

impl ClientStats {
    pub fn new(name: &str, app: &str) -> Self {
        Self {
            name: name.to_string(),
            app: app.to_string(),
            total: AtomicU64::new(0),
            last_total: AtomicU64::new(0),
            last_seen: AtomicI64::new(now_ms()),
            rate: AtomicU64::new(0),
            lat: Mutex::new(VecDeque::with_capacity(256)),
            history: Mutex::new(VecDeque::with_capacity(120)),
        }
    }
    pub fn rate_f64(&self) -> f64 {
        f64::from_bits(self.rate.load(std::sync::atomic::Ordering::Relaxed))
    }
    pub fn set_rate(&self, v: f64) {
        self.rate
            .store(v.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Adaptive limit state per app
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct LimitState {
    /// internal adaptive value (before multiplier/weight)
    pub internal: f64,
    /// enforced limit (what requests actually get)
    pub enforced: u32,
    // last breakdown for the dashboard
    pub lat_err: f64,
    pub pressure: f64,
    pub shrink: f64,
    pub p50_ms: f64,
    pub rate: f64,
    pub updated_ms: i64,
}

// ---------------------------------------------------------------------------
// System metrics snapshot
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct SystemSnapshot {
    pub cpu_pct: f64,
    pub mem_pct: f64,
    pub mem_used_mb: u64,
    pub mem_total_mb: u64,
    pub disk_pct: f64,
    pub disk_used_mb: u64,
    pub disk_total_mb: u64,
    pub net_rx_kbps: f64,
    pub net_tx_kbps: f64,
    pub uptime_s: u64,
    pub ts_ms: i64,
}

// ---------------------------------------------------------------------------
// Cursor registry
// ---------------------------------------------------------------------------

pub struct CursorInfo {
    pub id: String,
    pub db: String,
    pub coll: String,
    pub created_ms: i64,
    pub last_used_ms: AtomicI64,
    pub uses: AtomicU64,
}

// ---------------------------------------------------------------------------
// Admin session
// ---------------------------------------------------------------------------

pub struct AdminSession {
    pub user: String,
    pub expires_ms: i64,
}

// ---------------------------------------------------------------------------
// Log ring
// ---------------------------------------------------------------------------

pub fn log_push(state: &AppState, line: String) {
    if let Ok(mut logs) = state.logs.lock() {
        if logs.len() >= 1500 {
            logs.pop_front();
        }
        logs.push_back(line);
    }
}
