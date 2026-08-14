//! Central application state shared across all Tokio workers.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicI64, AtomicU64};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
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

/// One ring entry: the raw formatted line plus parsed fields the dashboard
/// uses for client-side filtering (severity, logger, app_id, name_id).
#[derive(Clone)]
pub struct LogEntry {
    /// Monotonic id — stable even when old entries are evicted from the ring.
    pub seq: u64,
    /// Fully formatted line: `2026-08-14T13:59:50.912518Z  INFO XavierDB: msg`.
    pub raw: String,
    /// INFO / WARN / ERROR / DEBUG / TRACE.
    pub level: String,
    /// tracing target (module path) or "XavierDB" for log_line lines.
    pub logger: String,
    pub app: Option<String>,
    pub name: Option<String>,
}

pub const LOG_CAP: usize = 3000;

struct LogRing {
    entries: VecDeque<LogEntry>,
    next_seq: u64,
    cap: usize,
}

impl LogRing {
    fn new(cap: usize) -> Self {
        Self { entries: VecDeque::with_capacity(cap), next_seq: 0, cap }
    }
    fn push(
        &mut self,
        level: String,
        logger: String,
        app: Option<String>,
        name: Option<String>,
        raw: String,
    ) {
        if self.entries.len() >= self.cap {
            self.entries.pop_front();
        }
        self.entries.push_back(LogEntry { seq: self.next_seq, raw, level, logger, app, name });
        self.next_seq += 1;
    }
}

/// Global ring (not on AppState) so lines emitted before the state exists
/// (env bootstrap, config/perms load, JWT notice, panic hook) are captured too.
static LOG_RING: OnceLock<Mutex<LogRing>> = OnceLock::new();

fn ring() -> &'static Mutex<LogRing> {
    LOG_RING.get_or_init(|| Mutex::new(LogRing::new(LOG_CAP)))
}

/// RFC3339 UTC with microsecond precision, matching tracing's default timer
/// (e.g. `2026-08-14T13:59:50.912518Z`). No chrono dependency: civil-date
/// conversion via Howard Hinnant's days-from-civil algorithm.
fn fmt_ts() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs();
    let micros = d.subsec_micros();
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let dd = doy - (153 * mp + 2) / 5 + 1;
    let mm = if mp < 10 { mp + 3 } else { mp - 9 };
    let yy = if mm <= 2 { y + 1 } else { y };
    format!("{yy:04}-{mm:02}-{dd:02}T{h:02}:{mi:02}:{s:02}.{micros:06}Z")
}

fn push_fmt(level: &str, app: Option<String>, name: Option<String>, msg: &str) {
    let raw = format!("{} {:>5} XavierDB: {msg}", fmt_ts(), level);
    if let Ok(mut r) = ring().lock() {
        r.push(level.to_string(), "XavierDB".to_string(), app, name, raw);
    }
}

/// Log a line to the ring AND stderr (console keeps the current behavior).
pub fn log_line(level: &str, msg: &str) {
    push_fmt(level, None, None, msg);
    eprintln!("{msg}");
}

/// Log a line to the ring AND stdout (used for one-shot bootstrap prints).
pub fn log_stdout(level: &str, msg: &str) {
    push_fmt(level, None, None, msg);
    println!("{msg}");
}

/// Push a fully formatted tracing line (LogWriter). Parses the level, the
/// logger (tracing target) and the auth identity (app/name) out of it.
pub fn log_push_raw(line: String) {
    let (level, logger, msg) = parse_level_msg(&line);
    let (name, app) = log_identify(&msg);
    if let Ok(mut r) = ring().lock() {
        r.push(level, logger, app, name, line);
    }
}

/// "2026-08-14T13:59:50.912518Z  INFO XavierDB::routes_misc: msg" -> ("INFO", "XavierDB::routes_misc", "msg")
fn parse_level_msg(line: &str) -> (String, String, String) {
    let rest = line.split_once(' ').map(|(_, r)| r.trim_start()).unwrap_or(line);
    let (lvl, rest) = rest.split_once(' ').unwrap_or((rest, ""));
    let (logger, msg) = rest
        .split_once(": ")
        .map(|(l, m)| (l.to_string(), m.to_string()))
        .unwrap_or_else(|| ("XavierDB".to_string(), rest.to_string()));
    (lvl.to_string(), logger, msg)
}

/// Extract (name, app) from identity-carrying log messages:
/// auth lines ("login OK: name@app") and per-request debug lines
/// ("GET /q/db/coll as name@app"); any other shape yields (None, None).
/// Admin logins are excluded.
fn log_identify(msg: &str) -> (Option<String>, Option<String>) {
    let rest = ["login OK: ", "login failed: ", "login blocked: "]
        .iter()
        .find_map(|p| msg.strip_prefix(p))
        .unwrap_or("")
        .trim();
    let id = if let Some((n, a)) = rest.rsplit_once('@') {
        if !n.is_empty() && !a.is_empty() && !a.contains(' ') {
            Some((n, a))
        } else {
            None
        }
    } else {
        None
    };
    let id = id.or_else(|| {
        // "... as name@app"
        let rest = msg.rsplit_once(" as ").map(|(_, r)| r.trim()).unwrap_or("");
        rest.rsplit_once('@').and_then(|(n, a)| {
            if !n.is_empty() && !a.is_empty() && !a.contains(' ') {
                Some((n, a))
            } else {
                None
            }
        })
    });
    match id {
        Some((n, a)) => (Some(n.to_string()), Some(a.to_string())),
        None => (None, None),
    }
}

/// Snapshot of the ring: entries in chronological order (oldest first).
/// `limit` = max entries (0 = all); `before` = only entries with seq < before.
/// Also returns the total count and the distinct app/name/logger facets for
/// the dashboard filter dropdowns.
pub fn log_snapshot(
    limit: usize,
    before: Option<u64>,
) -> (Vec<LogEntry>, u64, Vec<String>, Vec<String>, Vec<String>) {
    let r = ring().lock().unwrap();
    let total = r.entries.len() as u64;
    let mut apps: Vec<String> = r
        .entries
        .iter()
        .filter_map(|e| e.app.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let mut names: Vec<String> = r
        .entries
        .iter()
        .filter_map(|e| e.name.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let mut loggers: Vec<String> = r
        .entries
        .iter()
        .map(|e| e.logger.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    apps.sort();
    names.sort();
    loggers.sort();
    let iter = r.entries.iter().filter(|e| before.is_none_or(|b| e.seq < b));
    let v: Vec<LogEntry> = if limit == 0 {
        iter.cloned().collect()
    } else {
        iter.rev().take(limit).collect::<Vec<_>>().into_iter().rev().cloned().collect()
    };
    (v, total, apps, names, loggers)
}

// ---------------------------------------------------------------------------
// Log level knob (dashboard config dashboard.log_level, hot-reloadable)
// ---------------------------------------------------------------------------

/// Hook installed by main() once the tracing subscriber exists; lets config
/// reloads (dashboard save / reload-from-disk / file watcher) change the
/// verbosity (INFO vs DEBUG) without a restart.
static LOG_LEVEL_HOOK: OnceLock<Box<dyn Fn(&str) + Send + Sync>> = OnceLock::new();

pub fn set_log_level_hook(hook: Box<dyn Fn(&str) + Send + Sync>) {
    let _ = LOG_LEVEL_HOOK.set(hook);
}

pub fn apply_log_level(level: &str) {
    if let Some(h) = LOG_LEVEL_HOOK.get() {
        h(level);
    }
}
