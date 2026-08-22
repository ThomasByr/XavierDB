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
    /// Dashboard login throttle per IP (separate from /auth: server.yml
    /// admin.max_logins_per_ip_per_minute, default 5).
    pub dash_throttle: DashMap<String, (i64, u32)>,

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
    /// Admin dashboard credentials from server.yml.
    pub admin_user: String,
    pub password_hash: String,
    /// Max documents per insert batch (server.yml runtime.max_insert_batch,
    /// default 1000).
    pub max_insert_batch: usize,
    /// Dashboard login limit per IP per minute (server.yml
    /// admin.max_logins_per_ip_per_minute, default 5).
    pub dash_login_max_per_min: u32,
    /// Trust X-Real-IP / X-Forwarded-For for the client IP (server.yml
    /// network.trust_proxy_headers; enable only behind a reverse proxy —
    /// see settings.rs).
    pub trust_proxy_headers: bool,
    /// Server-side deadline for MongoDB find queries in ms (server.yml
    /// runtime.find_timeout_ms; 0 = disabled). Enforced in dbq::find_page.
    pub find_timeout_ms: u64,
    /// Keyset-pagination type-bracket mode (server.yml
    /// runtime.keyset_type_brackets; see dbq::KeysetTypeBrackets).
    pub keyset_type_brackets: crate::dbq::KeysetTypeBrackets,
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
        password_hash: String,
        max_insert_batch: usize,
        dash_login_max_per_min: u32,
        trust_proxy_headers: bool,
        find_timeout_ms: u64,
        keyset_type_brackets: crate::dbq::KeysetTypeBrackets,
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
            dash_throttle: DashMap::new(),
            mongo,
            started: Instant::now(),
            last_config_written: Mutex::new(None),
            last_perms_written: Mutex::new(None),
            cfg_version: AtomicU64::new(0),
            perms_version: AtomicU64::new(0),
            config_path,
            perms_path,
            admin_user,
            password_hash,
            max_insert_batch,
            dash_login_max_per_min,
            trust_proxy_headers,
            find_timeout_ms,
            keyset_type_brackets,
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

/// Rotating log files on disk — the ONLY log store (no in-memory ring: the
/// Logs tab reads back from these files, so memory stays flat regardless of
/// traffic). Settings come from server.yml (log.files / log.size_mb, see
/// server.yml.example) and are NOT live-changeable.
///
/// Files (cwd-relative): `xavierdb.log` = current/newest, `xavierdb.log.1` =
/// previous, ... `xavierdb.log.{files-1}` = oldest. When the current file
/// exceeds size_bytes it is renamed to .1 (shifting the chain, dropping the
/// oldest) and a fresh file is started.
const LOG_BASE: &str = "xavierdb.log";
/// Lines scanned for the filter facets on each /logs read (bounded: the
/// dropdowns never need the whole store).
const LOG_FACET_LINES: usize = 2000;

pub struct LogFileSink {
    dir: std::path::PathBuf,
    files: usize,
    size_bytes: u64,
    /// global line counter — seeded at init by scanning the existing files,
    /// so line numbers stay stable across restarts (and across rotations).
    next_seq: u64,
    cur_bytes: u64,
    /// Per-file line counts, tracked so reads never rescan the whole store
    /// (a paged read used to count + fully read every file per request,
    /// which made server RSS climb with every older-page search).
    /// `cur_lines` = current file; `rotated_lines[i - 1]` = suffix `i`.
    cur_lines: u64,
    rotated_lines: Vec<u64>,
    file: Option<std::fs::File>,
}

impl LogFileSink {
    fn new(dir: std::path::PathBuf, files: usize, size_bytes: u64) -> Self {
        let mut s = Self {
            dir,
            files: files.clamp(1, 10),
            size_bytes: size_bytes.max(1024),
            next_seq: 0,
            cur_bytes: 0,
            cur_lines: 0,
            rotated_lines: Vec::new(),
            file: None,
        };
        // seed the counters from existing files (restart continuity) and
        // reopen the current file in append mode
        let mut rotated = vec![0u64; s.files.saturating_sub(1)];
        for i in (1..s.files).rev() {
            rotated[i - 1] = count_lines(&s.dir.join(format!("{LOG_BASE}.{i}")));
        }
        let cur = s.dir.join(LOG_BASE);
        s.cur_lines = count_lines(&cur);
        s.next_seq = rotated.iter().sum::<u64>() + s.cur_lines;
        s.rotated_lines = rotated;
        s.cur_bytes = std::fs::metadata(&cur).map(|m| m.len()).unwrap_or(0);
        s.file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&cur)
            .ok();
        s
    }

    fn write(&mut self, line: &str) {
        if self.file.is_none() {
            return;
        }
        let add = line.len() as u64 + 1; // + newline
        if self.cur_bytes + add > self.size_bytes {
            self.rotate();
        }
        use std::io::Write;
        if let Some(f) = &mut self.file {
            if f.write_all(line.as_bytes())
                .and_then(|_| f.write_all(b"\n"))
                .is_ok()
            {
                self.cur_bytes += add;
                self.next_seq += 1;
                self.cur_lines += 1;
            }
        }
    }

    fn rotate(&mut self) {
        self.file = None; // close before renaming (Windows locks open files)
        let _ = std::fs::remove_file(self.dir.join(format!("{LOG_BASE}.{}", self.files - 1)));
        for i in (1..self.files - 1).rev() {
            let from = self.dir.join(format!("{LOG_BASE}.{i}"));
            let to = self.dir.join(format!("{LOG_BASE}.{}", i + 1));
            let _ = std::fs::rename(&from, &to);
        }
        let cur = self.dir.join(LOG_BASE);
        let _ = std::fs::rename(&cur, self.dir.join(format!("{LOG_BASE}.1")));
        // shift the tracked counts with the files: .i -> .i+1, current -> .1
        if self.files > 1 {
            for i in (1..self.files - 1).rev() {
                self.rotated_lines[i] = self.rotated_lines[i - 1];
            }
            self.rotated_lines[0] = self.cur_lines;
        }
        self.cur_lines = 0;
        self.cur_bytes = 0;
        self.file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&cur)
            .ok();
    }

    /// Entries in chronological order (oldest first). `limit` = max entries
    /// (0 = all); `before` = only entries with seq < before. Facets come from
    /// the last `max(limit, LOG_FACET_LINES)` lines (bounded scan).
    fn read(
        &self,
        limit: usize,
        before: Option<u64>,
    ) -> (
        Vec<LogEntry>,
        u64,
        Vec<String>,
        Vec<(String, String)>,
        Vec<String>,
    ) {
        let total = self.next_seq;
        // files oldest -> newest with their tracked line counts
        let mut files: Vec<(std::path::PathBuf, u64)> = Vec::new();
        for i in (1..self.files).rev() {
            let p = self.dir.join(format!("{LOG_BASE}.{i}"));
            if p.exists() {
                files.push((p, self.rotated_lines[i - 1]));
            }
        }
        let cur = self.dir.join(LOG_BASE);
        if cur.exists() {
            files.push((cur, self.cur_lines));
        }
        // seq of the first line of each file (prefix sums over oldest->newest)
        let mut base = 0u64;
        let mut bases = Vec::with_capacity(files.len());
        for (_, c) in &files {
            bases.push(base);
            base = base.saturating_add(*c);
        }
        let mut collected: Vec<LogEntry> = Vec::new();
        let mut want = if limit == 0 {
            usize::MAX
        } else {
            limit.max(LOG_FACET_LINES)
        };
        // Walk newest -> oldest, collecting the newest `want` entries below
        // `before`. Memory-bounded: files that cannot contribute are never
        // opened (entirely at/after `before`, or `want` already satisfied),
        // and each opened file is read as a TAIL WINDOW ONLY (read_tail
        // locates the byte range by counting newlines backwards in bounded
        // chunks, then reads just that range) — a paged request holds the
        // bytes of its ≤ max(limit, LOG_FACET_LINES) lines, never a whole
        // multi-MB file.
        'walk: for (idx, (p, count)) in files.iter().enumerate().rev() {
            if want == 0 {
                break;
            }
            if *count == 0 {
                continue;
            }
            let base_seq = bases[idx];
            if before.is_some_and(|b| base_seq >= b) {
                continue; // whole file lives at/after `before` — skip it
            }
            // tail lines with seq >= before (only the boundary file has any)
            let skip_end = before
                .map(|b| count.saturating_sub(b - base_seq))
                .unwrap_or(0);
            let take = want.min((*count - skip_end) as usize);
            let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            let Some(window) = read_tail(p, skip_end, take as u64, size) else {
                continue;
            };
            // window holds exactly the `take` needed lines (oldest->newest
            // in the String) — rev() yields them newest-first
            for (k, line) in window.lines().rev().enumerate() {
                // k < take <= count - skip_end keeps seq >= base_seq
                let seq = base_seq + count - 1 - skip_end - k as u64;
                let line = line.strip_suffix('\r').unwrap_or(line);
                let (level, logger, msg) = parse_level_msg(line);
                let (name, app) = log_identify(&msg);
                collected.push(LogEntry {
                    seq,
                    raw: line.to_string(),
                    level,
                    logger,
                    app,
                    name,
                });
                want -= 1;
                if want == 0 {
                    break 'walk;
                }
            }
        }
        collected.reverse(); // oldest -> newest
        // facets from the whole collected window; response = last `limit`
        let mut apps: Vec<String> = collected
            .iter()
            .filter_map(|e| e.app.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let mut names: Vec<(String, String)> = collected
            .iter()
            .filter_map(|e| {
                e.app
                    .as_ref()
                    .zip(e.name.as_ref())
                    .map(|(a, n)| (a.clone(), n.clone()))
            })
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let mut loggers: Vec<String> = collected
            .iter()
            .map(|e| e.logger.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        apps.sort();
        names.sort();
        loggers.sort();
        let cut = if limit == 0 {
            0
        } else {
            collected.len().saturating_sub(limit)
        };
        let entries = collected.split_off(cut);
        (entries, total, apps, names, loggers)
    }

    fn retention(&self) -> (u32, u32, String) {
        (
            self.files as u32,
            (self.size_bytes / (1024 * 1024)) as u32,
            LOG_BASE.to_string(),
        )
    }
}

/// Global sink (not on AppState) so lines emitted before the state exists
/// (env bootstrap, config/perms load, JWT notice, panic hook) are captured too.
static LOG_FILES: OnceLock<Mutex<LogFileSink>> = OnceLock::new();

/// Initialize the rotating log files. Called once at startup from server.yml
/// log.files / log.size_mb (clamped to 1..=10 / 1..=20).
pub fn init_log_files(files: usize, size_mb: usize) {
    let dir = std::env::current_dir().unwrap_or_default();
    let sink = LogFileSink::new(dir, files, (size_mb as u64) * 1024 * 1024);
    let _ = LOG_FILES.set(Mutex::new(sink));
}

/// Append a fully formatted line to the rotating files (no parsing at write
/// time — lines are parsed on read for the structured /logs payload).
pub fn log_file_write(line: &str) {
    if let Some(m) = LOG_FILES.get() {
        let mut s = m.lock().unwrap_or_else(|p| p.into_inner());
        s.write(line);
    }
}

/// Read a window of log entries from the files (see LogFileSink::read).
pub fn log_read(
    limit: usize,
    before: Option<u64>,
) -> (
    Vec<LogEntry>,
    u64,
    Vec<String>,
    Vec<(String, String)>,
    Vec<String>,
) {
    match LOG_FILES.get() {
        Some(m) => match m.lock() {
            Ok(s) => s.read(limit, before),
            Err(p) => p.into_inner().read(limit, before),
        },
        None => (Vec::new(), 0, Vec::new(), Vec::new(), Vec::new()),
    }
}

pub fn log_retention() -> (u32, u32, String) {
    match LOG_FILES.get() {
        Some(m) => m
            .lock()
            .map(|s| s.retention())
            .unwrap_or((0, 0, String::new())),
        None => (0, 0, String::new()),
    }
}

fn count_lines(p: &std::path::Path) -> u64 {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(p) else {
        return 0;
    };
    let mut buf = [0u8; 64 * 1024];
    let mut n = 0u64;
    loop {
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(k) => n += buf[..k].iter().filter(|&&b| b == b'\n').count() as u64,
            Err(_) => break,
        }
    }
    n
}

/// Read the `take` lines that start `skip` lines from the end of `p`
/// (`size` = its byte length), as one String (oldest -> newest). Locates the
/// window by counting newlines BACKWARDS in bounded chunks and then reads
/// only that byte range, so serving a page from a huge file never holds the
/// whole file in memory — only the requested lines' bytes (plus one 64 KiB
/// scan chunk). The sink always terminates lines with '\n'; a hand-appended
/// final line without one still counts (missing trailing newline = one
/// boundary at EOF). Invalid UTF-8 degrades lossily instead of failing the
/// whole read.
fn read_tail(p: &std::path::Path, skip: u64, take: u64, size: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let need = skip.checked_add(take)?;
    if need == 0 {
        return Some(String::new());
    }
    let mut f = std::fs::File::open(p).ok()?;
    // Start of the k-th line from the end = offset just after the (k+1)-th
    // boundary from the end (a boundary being a '\n', plus a phantom one at
    // BOF, and one at EOF when the file lacks a trailing newline). So the
    // window [o1, o2) = lines (skip .. skip+take] from the end is bounded by
    // boundary #(need+1) and boundary #(skip+1).
    let mut passed = 0u64;
    let mut o1: u64 = 0; // start of the first needed line (BOF phantom unless found)
    let mut o2: u64 = size; // end boundary: start of the line `skip` from the end
    let mut set_o2 = skip == 0; // skip == 0 -> the window ends at EOF
    if size > 0 {
        f.seek(SeekFrom::Start(size - 1)).ok()?;
        let mut last = [0u8; 1];
        f.read_exact(&mut last).ok()?;
        if last[0] != b'\n' {
            passed = 1; // phantom boundary at EOF terminates the partial last line
        }
    }
    if passed < need + 1 {
        let mut pos = size;
        let mut buf = [0u8; 64 * 1024];
        'scan: while pos > 0 {
            let chunk = (pos.min(buf.len() as u64)) as usize;
            let at = pos - chunk as u64;
            f.seek(SeekFrom::Start(at)).ok()?;
            f.read_exact(&mut buf[..chunk]).ok()?;
            for i in (0..chunk).rev() {
                if buf[i] == b'\n' {
                    passed += 1;
                    let after = at + i as u64 + 1;
                    if !set_o2 && passed == skip + 1 {
                        o2 = after;
                        set_o2 = true;
                    }
                    if passed == need + 1 {
                        o1 = after;
                        break 'scan;
                    }
                }
            }
            pos = at;
        }
        // fewer lines on disk than tracked (external truncation): window
        // clamps to [BOF, o2)
    }
    if o1 >= o2 {
        return Some(String::new());
    }
    let mut bytes = vec![0u8; (o2 - o1) as usize];
    f.seek(SeekFrom::Start(o1)).ok()?;
    f.read_exact(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
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

fn push_fmt(level: &str, msg: &str) {
    let raw = format!("{} {:>5} XavierDB: {msg}", fmt_ts(), level);
    log_file_write(&raw);
}

/// Log a line to the rotating files AND stderr (console keeps its behavior).
pub fn log_line(level: &str, msg: &str) {
    push_fmt(level, msg);
    eprintln!("{msg}");
}

/// Log a line to the rotating files AND stdout (used for one-shot bootstrap prints).
pub fn log_stdout(level: &str, msg: &str) {
    push_fmt(level, msg);
    println!("{msg}");
}

/// Append a fully formatted tracing line (LogWriter) to the files. Identity
/// and level are parsed on read (see log_identify / parse_level_msg).
pub fn log_push_raw(line: String) {
    log_file_write(&line);
}

/// "2026-08-14T13:59:50.912518Z  INFO XavierDB::routes_misc: msg" -> ("INFO", "XavierDB::routes_misc", "msg")
fn parse_level_msg(line: &str) -> (String, String, String) {
    let rest = line
        .split_once(' ')
        .map(|(_, r)| r.trim_start())
        .unwrap_or(line);
    let (lvl, rest) = rest.split_once(' ').unwrap_or((rest, ""));
    let (logger, msg) = rest
        .split_once(": ")
        .map(|(l, m)| (l.to_string(), m.to_string()))
        .unwrap_or_else(|| ("XavierDB".to_string(), rest.to_string()));
    (lvl.to_string(), logger, msg)
}

/// Extract (name, app) from identity-carrying log messages:
/// auth lines ("login OK: name@app from 1.2.3.4:5678") and per-request
/// debug lines ("GET /q/db/coll from 1.2.3.4:5678 as name@app"); any other
/// shape yields (None, None). Admin logins are excluded.
fn log_identify(msg: &str) -> (Option<String>, Option<String>) {
    let rest = ["login OK: ", "login failed: ", "login blocked: "]
        .iter()
        .find_map(|p| msg.strip_prefix(p))
        .unwrap_or("")
        .trim();
    let id = if let Some((n, a)) = rest.rsplit_once('@') {
        // strip the trailing " from <peer addr>" login lines carry
        let a = a.split_once(" from ").map(|(x, _)| x).unwrap_or(a);
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

/// Snapshot of the log files: entries in chronological order (oldest first).
/// `limit` = max entries (0 = all); `before` = only entries with seq < before.
/// Also returns the total count and the distinct app / (app,name) / logger
/// facets for the dashboard filter dropdowns.
pub fn log_snapshot(
    limit: usize,
    before: Option<u64>,
) -> (
    Vec<LogEntry>,
    u64,
    Vec<String>,
    Vec<(String, String)>,
    Vec<String>,
) {
    log_read(limit, before)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_identify_formats() {
        // current formats (with peer addr) ...
        assert_eq!(
            log_identify("login OK: u1@app1 from 1.2.3.4:5678"),
            (Some("u1".into()), Some("app1".into()))
        );
        assert_eq!(
            log_identify("login failed: u1@app1 from [::1]:443"),
            (Some("u1".into()), Some("app1".into()))
        );
        assert_eq!(
            log_identify("GET /q/db/coll from 127.0.0.1:9999 as u2@app2"),
            (Some("u2".into()), Some("app2".into()))
        );
        // ... and legacy lines from rotated files (no addr)
        assert_eq!(
            log_identify("login OK: u1@app1"),
            (Some("u1".into()), Some("app1".into()))
        );
        assert_eq!(
            log_identify("GET /q/db/coll as u2@app2"),
            (Some("u2".into()), Some("app2".into()))
        );
        // non-identity lines
        assert_eq!(log_identify("login throttled: 1.2.3.4:5678"), (None, None));
        assert_eq!(log_identify("server started"), (None, None));
    }

    #[test]
    fn log_files_rotate_bounded_and_keep_order() {
        let dir = std::env::temp_dir().join(format!("xdb-logtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 3 files × 120 bytes: ~6 lines of ~50 bytes per file before rotating
        let mut s = LogFileSink::new(dir.clone(), 3, 120);
        for i in 0..40 {
            s.write(&format!(
                "line {i:04} pppppppppppppppppppppppppppppppppppppppp"
            ));
        }
        drop(s);

        // reopen: seq continuity across "restart"
        let s2 = LogFileSink::new(dir.clone(), 3, 120);
        let (entries, total, _, _, _) = s2.read(0, None);
        assert_eq!(total, 40, "total line count");
        assert_eq!(entries.len(), 40, "all lines readable");
        assert_eq!(
            entries[0].raw,
            "line 0000 pppppppppppppppppppppppppppppppppppppppp"
        );
        assert_eq!(
            entries[39].raw,
            "line 0039 pppppppppppppppppppppppppppppppppppppppp"
        );
        assert!(
            entries.windows(2).all(|w| w[0].seq + 1 == w[1].seq),
            "seqs contiguous"
        );
        assert_eq!(entries[0].seq, 0, "seqs restart at 0 on a fresh store");

        // bounded: at most `files` files on disk
        let n_files = (0..=3)
            .filter(|i| {
                let p = if *i == 0 {
                    dir.join(LOG_BASE)
                } else {
                    dir.join(format!("{LOG_BASE}.{i}"))
                };
                p.exists()
            })
            .count();
        assert!(n_files <= 3, "file count bounded: {n_files}");

        // paging: before=<seq> returns only older lines
        let (page, _, _, _, _) = s2.read(5, Some(10));
        assert_eq!(page.len(), 5);
        assert!(page.iter().all(|e| e.seq < 10));
        assert_eq!(page[0].seq, 5, "last 5 before seq 10");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn log_read_older_walk_reassembles_store() {
        let dir = std::env::temp_dir().join(format!("xdb-logwalk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = LogFileSink::new(dir.clone(), 3, 240);
        for i in 0..40 {
            s.write(&format!(
                "walk {i:04} pppppppppppppppppppppppppppppppppppppppp"
            ));
        }
        // walk the whole store page by page, exactly like the dashboard's
        // "searching older logs" auto-pager: newest page, then before=<oldest>
        let mut seen: Vec<u64> = Vec::new();
        let mut before: Option<u64> = None;
        loop {
            let (page, total, _, _, _) = s.read(7, before);
            assert_eq!(total, 40);
            assert!(before.is_none_or(|b| page.iter().all(|e| e.seq < b)));
            let Some(oldest) = page.first().map(|e| e.seq) else {
                break;
            };
            seen.extend(page.iter().map(|e| e.seq));
            if oldest == 0 {
                break;
            }
            before = Some(oldest);
        }
        seen.sort(); // order-insensitive: every seq must appear exactly once
        let expect: Vec<u64> = (0..40).collect();
        assert_eq!(seen, expect, "older-page walk covers every seq exactly once");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn log_sink_tracks_per_file_counts_across_rotations() {
        let dir = std::env::temp_dir().join(format!("xdb-logcnt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = LogFileSink::new(dir.clone(), 3, 120);
        for i in 0..40 {
            s.write(&format!(
                "line {i:04} pppppppppppppppppppppppppppppppppppppppp"
            ));
        }
        // tracked counts must always match what a fresh scan of disk sees
        let tracked: u64 = s.rotated_lines.iter().sum::<u64>() + s.cur_lines;
        assert_eq!(tracked, s.next_seq, "tracked counts sum to next_seq");
        let on_disk: u64 = (0..=2)
            .map(|i| {
                let p = if i == 0 {
                    dir.join(LOG_BASE)
                } else {
                    dir.join(format!("{LOG_BASE}.{i}"))
                };
                count_lines(&p)
            })
            .sum();
        assert_eq!(tracked, on_disk, "tracked counts match disk after rotations");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn log_read_tail_windows_large_files() {
        let dir = std::env::temp_dir().join(format!("xdb-logtail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 2000 lines × ~100 bytes = ~200 KB: windows must span several of
        // read_tail's 64 KiB scan chunks in both directions
        let lines: Vec<String> = (0..2000)
            .map(|i| format!("{:04} {}", i, "x".repeat(95)))
            .collect();
        let p = dir.join(LOG_BASE);
        std::fs::write(&p, lines.join("\n") + "\n").unwrap();
        let size = std::fs::metadata(&p).unwrap().len();
        let want = |skip: u64, take: u64| {
            read_tail(&p, skip, take, size)
                .unwrap()
                .lines()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
        };
        // newest window
        assert_eq!(want(0, 3), lines[1997..2000]);
        // middle window across chunk boundaries
        assert_eq!(want(500, 300), lines[1200..1500]);
        // oldest window (o1 clamps to BOF)
        assert_eq!(want(1997, 3), lines[0..3]);
        // whole file (take = all)
        assert_eq!(want(0, 2000).len(), 2000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn log_read_tail_handles_missing_trailing_newline() {
        let dir = std::env::temp_dir().join(format!("xdb-lognl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(LOG_BASE);
        std::fs::write(&p, "aaa\nbbb\nccc").unwrap(); // no trailing newline
        let size = std::fs::metadata(&p).unwrap().len();
        // the partial last line still counts as a line
        assert_eq!(read_tail(&p, 0, 1, size).unwrap(), "ccc");
        assert_eq!(read_tail(&p, 1, 1, size).unwrap(), "bbb\n");
        assert_eq!(read_tail(&p, 2, 1, size).unwrap(), "aaa\n");
        assert_eq!(read_tail(&p, 0, 3, size).unwrap(), "aaa\nbbb\nccc");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
