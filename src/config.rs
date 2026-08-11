//! Binary configuration file with checksum, atomic writes, backups,
//! undo/redo history and reload support.
//!
//! File layout on disk:
//!   [4 bytes magic "XDB1"] [4 bytes crc32 of payload] [4 bytes payload len] [payload: bincode(ConfigFile)]

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const CONFIG_MAGIC: &[u8; 4] = b"XDB1";
pub const CONFIG_VERSION: u32 = 1;
pub const HISTORY_CAPACITY: usize = 10_000;
pub const MAX_BACKUPS: usize = 5;

// ---------------------------------------------------------------------------
// Structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GlobalCfg {
    /// Lifetime of issued JWTs, in minutes.
    pub jwt_token_lifetime_minutes: u64,
    /// Path of the permissions file (relative to the working directory).
    pub permission_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RateLimitCfg {
    /// Lower bound of the per-request document limit.
    pub min_limit: u32,
    /// Upper bound of the per-request document limit.
    pub max_limit: u32,
    /// Global coefficient applied on top of the adaptive limit
    /// (the "master dial" of the rate limiter).
    pub multiplier: f64,
    /// Target p50 processing latency in ms; above it, limits shrink.
    pub target_latency_ms: f64,
    /// How strongly system pressure (CPU/RAM) shrinks limits.
    pub pressure_sensitivity: f64,
    /// How strongly latency overshoot shrinks limits.
    pub latency_sensitivity: f64,
    /// Growth factor per tick when the system is healthy (slow recovery).
    pub growth_rate: f64,
    /// Interval in seconds between two adaptive-limit recomputations.
    pub tick_seconds: u64,
    /// EMA smoothing factor for request-rate measurements (0..1).
    pub ema_alpha: f64,
    /// Per-app weight multipliers (app_id -> weight).
    pub weights: HashMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthCfg {
    /// How long a /health answer stays cached, in seconds.
    pub cache_ttl_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DashboardCfg {
    /// How often the browser polls metrics, in seconds.
    pub poll_seconds: u64,
    /// Number of samples used for client-side graph smoothing.
    pub graph_smoothing: u32,
    /// "system" | "light" | "dark"
    pub theme: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthCfg {
    /// Max /auth attempts per minute per IP (brute-force protection).
    pub max_per_minute_per_ip: u32,
    /// Admin dashboard session lifetime in hours.
    pub session_ttl_hours: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryEntry {
    pub ts: i64,
    pub desc: String,
    /// dotted field path, e.g. "rate_limit.multiplier"
    pub path: String,
    /// full bincode snapshot of the config before the change
    pub snapshot: Vec<u8>,
    pub by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfigFile {
    pub version: u32,
    pub created_at: i64,
    pub last_modified: i64,
    pub global: GlobalCfg,
    pub rate_limit: RateLimitCfg,
    pub health: HealthCfg,
    pub dashboard: DashboardCfg,
    pub auth: AuthCfg,
    /// blocked identifiers: "name@app" or bare "app"
    pub blocked: Vec<String>,
    /// undo history, oldest first (snapshot = state BEFORE the change)
    pub history: Vec<HistoryEntry>,
    /// redo stack, most recent first (snapshot = state AFTER the undone change)
    pub redo: Vec<HistoryEntry>,
}

impl Default for ConfigFile {
    fn default() -> Self {
        let now = now_secs();
        Self {
            version: CONFIG_VERSION,
            created_at: now,
            last_modified: now,
            global: GlobalCfg {
                jwt_token_lifetime_minutes: 90,
                permission_file: "authorized_keys.yml".to_string(),
            },
            rate_limit: RateLimitCfg {
                min_limit: 1,
                max_limit: 200,
                multiplier: 1.0,
                target_latency_ms: 50.0,
                pressure_sensitivity: 1.5,
                latency_sensitivity: 1.0,
                growth_rate: 1.15,
                tick_seconds: 5,
                ema_alpha: 0.2,
                weights: HashMap::new(),
            },
            health: HealthCfg {
                cache_ttl_seconds: 5,
            },
            dashboard: DashboardCfg {
                poll_seconds: 2,
                graph_smoothing: 5,
                theme: "system".to_string(),
            },
            auth: AuthCfg {
                max_per_minute_per_ip: 30,
                session_ttl_hours: 24,
            },
            blocked: Vec::new(),
            history: Vec::new(),
            redo: Vec::new(),
        }
    }
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// (de)serialization
// ---------------------------------------------------------------------------

pub fn encode(cfg: &ConfigFile) -> Result<Vec<u8>, String> {
    bincode::serde::encode_to_vec(cfg, bincode::config::standard())
        .map_err(|e| format!("config encode failed: {e}"))
}

pub fn decode(bytes: &[u8]) -> Result<ConfigFile, String> {
    let (cfg, _) = bincode::serde::decode_from_slice(bytes, bincode::config::standard())
        .map_err(|e| format!("config decode failed: {e}"))?;
    Ok(cfg)
}

fn serialize_with_checksum(cfg: &ConfigFile) -> Result<Vec<u8>, String> {
    let payload = encode(cfg)?;
    let mut out = Vec::with_capacity(12 + payload.len());
    out.extend_from_slice(CONFIG_MAGIC);
    let crc = crc32fast::hash(&payload);
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

fn deserialize_with_checksum(bytes: &[u8]) -> Result<ConfigFile, String> {
    if bytes.len() < 12 || &bytes[0..4] != CONFIG_MAGIC {
        return Err("not a XavierDB config file (bad magic)".into());
    }
    let crc = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    if 12 + len > bytes.len() {
        return Err("config file truncated".into());
    }
    let payload = &bytes[12..12 + len];
    let actual = crc32fast::hash(payload);
    if actual != crc {
        return Err("config checksum mismatch (corrupted file)".into());
    }
    let cfg = decode(payload)?;
    if cfg.version != CONFIG_VERSION {
        // future-proofing: unknown schema version -> refuse (restore backup instead)
        return Err(format!(
            "unsupported config version {} (app supports {})",
            cfg.version, CONFIG_VERSION
        ));
    }
    Ok(cfg)
}

// ---------------------------------------------------------------------------
// file I/O with atomic writes and backup rotation
// ---------------------------------------------------------------------------

fn backup_paths(path: &Path) -> Vec<std::path::PathBuf> {
    (1..=MAX_BACKUPS)
        .map(|i| {
            if i == 1 {
                path.with_extension("bak")
            } else {
                path.with_extension(format!("bak.{}", i))
            }
        })
        .collect()
}

/// Encode the config WITHOUT history/redo payloads. Snapshots must stay flat:
/// nesting the history inside every snapshot doubles the file size on each
/// mutation (each entry embeds the whole past) and makes the 10k history cap
/// unreachable.
fn encode_flat(cfg: &ConfigFile) -> Vec<u8> {
    let mut c = cfg.clone();
    c.history.clear();
    c.redo.clear();
    encode(&c).unwrap_or_default()
}

fn rotate_backups(path: &Path) {
    // keep the chain config.bak (newest), config.bak.2, …, config.bak.MAX
    // (oldest): drop the oldest, shift the rest down, then copy the current
    // file into config.bak
    let oldest = path.with_extension(format!("bak.{}", MAX_BACKUPS));
    let _ = fs::remove_file(&oldest);
    for i in (2..MAX_BACKUPS).rev() {
        let from = path.with_extension(format!("bak.{}", i));
        let to = path.with_extension(format!("bak.{}", i + 1));
        if from.exists() {
            let _ = fs::rename(&from, &to);
        }
    }
    let from = path.with_extension("bak");
    let to = path.with_extension("bak.2");
    if from.exists() {
        let _ = fs::rename(&from, &to);
    }
    if path.exists() {
        let _ = fs::copy(path, path.with_extension("bak"));
    }
}

/// Atomically persist the config: temp file -> fsync -> rename, plus backups.
/// Returns the serialized bytes (used by the watcher to ignore our own writes).
pub fn save_to_disk(cfg: &ConfigFile, path: &Path) -> Result<Vec<u8>, String> {
    let bytes = serialize_with_checksum(cfg)?;
    rotate_backups(path);
    let tmp = path.with_extension("tmp");
    {
        let mut f =
            fs::File::create(&tmp).map_err(|e| format!("cannot create {}: {e}", tmp.display()))?;
        f.write_all(&bytes)
            .map_err(|e| format!("write failed: {e}"))?;
        f.sync_all().map_err(|e| format!("fsync failed: {e}"))?;
    }
    fs::rename(&tmp, path).map_err(|e| format!("rename failed: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(bytes)
}

/// Load from disk, falling back to backups and finally to defaults.
pub fn load_from_disk(path: &Path) -> (ConfigFile, Option<String>) {
    let mut candidates: Vec<std::path::PathBuf> = vec![path.to_path_buf()];
    candidates.extend(backup_paths(path));
    for c in candidates {
        match fs::read(&c) {
            Ok(bytes) => match deserialize_with_checksum(&bytes) {
                Ok(mut cfg) => {
                    // clamp to safe ranges: a corrupted or hand-tuned binary
                    // config must never feed unbounded sizes/lifetimes into
                    // request paths (or panic the metrics tick)
                    cfg.sanitize();
                    return (cfg, None);
                }
                Err(e) => eprintln!("[config] {} : {e} (trying backup)", c.display()),
            },
            Err(_) => continue,
        }
    }
    eprintln!("[config] no valid config found, generating defaults");
    let cfg = ConfigFile::default();
    let err = save_to_disk(&cfg, path)
        .err()
        .map(|e| format!("could not write default config: {e}"));
    (cfg, err)
}

// ---------------------------------------------------------------------------
// history / undo / redo
// ---------------------------------------------------------------------------

impl ConfigFile {
    /// Records the change and replaces `self` with `new`.
    pub fn apply(&mut self, new: ConfigFile, desc: &str, path: &str, by: &str) {
        let snapshot = encode_flat(self);
        let mut next = new;
        // keep the history (the incoming struct usually is a fresh clone)
        next.history = std::mem::take(&mut self.history);
        next.redo.clear();
        next.last_modified = now_secs();
        next.history.push(HistoryEntry {
            ts: now_secs(),
            desc: desc.to_string(),
            path: path.to_string(),
            snapshot,
            by: by.to_string(),
        });
        if next.history.len() > HISTORY_CAPACITY {
            let excess = next.history.len() - HISTORY_CAPACITY;
            next.history.drain(0..excess);
        }
        *self = next;
    }

    pub fn undo(&mut self) -> Option<HistoryEntry> {
        let entry = self.history.pop()?;
        // redo snapshot of the state we are undoing FROM, taken after the pop
        // but flat (no nested history); the popped entry is re-added to the
        // history by redo() from this entry's metadata
        let after = encode_flat(self);
        let mut redo = std::mem::take(&mut self.redo);
        redo.push(HistoryEntry {
            ts: now_secs(),
            desc: entry.desc.clone(),
            path: entry.path.clone(),
            snapshot: after,
            by: entry.by.clone(),
        });
        if let Ok(mut prev) = decode(&entry.snapshot) {
            prev.history = std::mem::take(&mut self.history);
            prev.redo = redo;
            prev.last_modified = now_secs();
            *self = prev;
            return Some(entry);
        }
        self.redo = redo;
        None
    }

    pub fn redo(&mut self) -> Option<HistoryEntry> {
        let entry = self.redo.pop()?;
        if let Ok(mut next) = decode(&entry.snapshot) {
            // snapshots are flat, so rebuild the history entry for this change:
            // the snapshot of the state before it is the current (undone) state
            let snap = encode_flat(self);
            next.history = std::mem::take(&mut self.history);
            next.history.push(HistoryEntry {
                ts: entry.ts,
                desc: entry.desc.clone(),
                path: entry.path.clone(),
                snapshot: snap,
                by: entry.by.clone(),
            });
            next.redo = std::mem::take(&mut self.redo);
            next.last_modified = now_secs();
            *self = next;
            return Some(entry);
        }
        None
    }

    /// Clamp every field to its safe range. Must always keep
    /// min_limit <= max_limit: the metrics loop clamps with both (min > max
    /// would panic the tick task). Applied on dashboard save/import/revert
    /// AND on load_from_disk: a corrupted or hand-tuned binary config must
    /// never feed unbounded sizes/lifetimes into request paths (e.g. a huge
    /// max_limit OOMs a paged query; a huge JWT lifetime overflows
    /// `lifetime as i64 * 60` on the unauthenticated /auth route).
    pub fn sanitize(&mut self) {
        self.version = CONFIG_VERSION;
        self.rate_limit.min_limit = self.rate_limit.min_limit.clamp(1, 10_000);
        self.rate_limit.max_limit = self.rate_limit.max_limit.max(self.rate_limit.min_limit);
        self.rate_limit.max_limit = self.rate_limit.max_limit.min(10_000);
        for w in self.rate_limit.weights.values_mut() {
            *w = w.clamp(0.1, 10.0);
        }
        self.rate_limit.multiplier = self.rate_limit.multiplier.clamp(0.05, 20.0);
        self.rate_limit.target_latency_ms = self.rate_limit.target_latency_ms.clamp(1.0, 60_000.0);
        self.rate_limit.growth_rate = self.rate_limit.growth_rate.clamp(1.0, 2.0);
        self.rate_limit.tick_seconds = self.rate_limit.tick_seconds.clamp(1, 3600);
        self.rate_limit.ema_alpha = self.rate_limit.ema_alpha.clamp(0.01, 0.9);
        self.rate_limit.pressure_sensitivity =
            self.rate_limit.pressure_sensitivity.clamp(0.0, 20.0);
        self.rate_limit.latency_sensitivity = self.rate_limit.latency_sensitivity.clamp(0.0, 20.0);
        self.health.cache_ttl_seconds = self.health.cache_ttl_seconds.clamp(1, 3600);
        self.dashboard.poll_seconds = self.dashboard.poll_seconds.clamp(1, 3600);
        self.dashboard.graph_smoothing = self.dashboard.graph_smoothing.clamp(1, 60);
        if !["system", "light", "dark"].contains(&self.dashboard.theme.as_str()) {
            self.dashboard.theme = "system".into();
        }
        self.auth.max_per_minute_per_ip = self.auth.max_per_minute_per_ip.clamp(1, 10_000);
        self.auth.session_ttl_hours = self.auth.session_ttl_hours.clamp(1, 24 * 30);
        self.global.jwt_token_lifetime_minutes = self
            .global
            .jwt_token_lifetime_minutes
            .clamp(1, 60 * 24 * 30);
    }

    pub fn history_meta(&self) -> Vec<(i64, String, String, String)> {
        self.history
            .iter()
            .rev()
            .map(|h| (h.ts, h.desc.clone(), h.path.clone(), h.by.clone()))
            .collect()
    }

    pub fn is_blocked(&self, id: &str) -> bool {
        self.blocked.iter().any(|b| b == id)
    }

    /// Whether `id` (name@app or app) is blocked, including app-level blocks.
    pub fn blocked_status(&self, name: &str, app: &str) -> BlockStatus {
        let full = format!("{name}@{app}");
        if self.is_blocked(&full) {
            BlockStatus::Name
        } else if self.is_blocked(app) {
            BlockStatus::App
        } else {
            BlockStatus::None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockStatus {
    None,
    Name,
    App,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let cfg = ConfigFile::default();
        let bytes = serialize_with_checksum(&cfg).unwrap();
        let back = deserialize_with_checksum(&bytes).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn detects_corruption() {
        let cfg = ConfigFile::default();
        let mut bytes = serialize_with_checksum(&cfg).unwrap();
        let n = bytes.len();
        bytes[n - 3] ^= 0xff;
        assert!(deserialize_with_checksum(&bytes).is_err());
    }

    #[test]
    fn undo_redo() {
        let mut cfg = ConfigFile::default();
        let mut a = cfg.clone();
        a.rate_limit.multiplier = 2.0;
        cfg.apply(a, "set multiplier", "rate_limit.multiplier", "test");
        assert_eq!(cfg.rate_limit.multiplier, 2.0);
        assert!(cfg.undo().is_some());
        assert_eq!(cfg.rate_limit.multiplier, 1.0);
        assert!(cfg.redo().is_some());
        assert_eq!(cfg.rate_limit.multiplier, 2.0);
    }

    #[test]
    fn undo_redo_keeps_history_entry() {
        let mut cfg = ConfigFile::default();
        let mut a = cfg.clone();
        a.rate_limit.multiplier = 2.0;
        cfg.apply(a, "mult 2", "rate_limit.multiplier", "test");
        let mut b = cfg.clone();
        b.rate_limit.multiplier = 3.0;
        cfg.apply(b, "mult 3", "rate_limit.multiplier", "test");
        assert_eq!(cfg.history.len(), 2);
        assert!(cfg.undo().is_some());
        assert_eq!(cfg.rate_limit.multiplier, 2.0);
        assert!(cfg.redo().is_some());
        assert_eq!(cfg.rate_limit.multiplier, 3.0);
        // the redone change must still be in history: the next undo reverts
        // exactly one change (back to 2.0), not two (to 1.0)
        assert_eq!(cfg.history.len(), 2);
        assert!(cfg.undo().is_some());
        assert_eq!(cfg.rate_limit.multiplier, 2.0);
    }

    #[test]
    fn history_snapshots_stay_flat() {
        let mut cfg = ConfigFile::default();
        let base = encode(&cfg).unwrap().len();
        for i in 1..=20 {
            let mut a = cfg.clone();
            a.rate_limit.multiplier = i as f64;
            cfg.apply(a, "tick", "rate_limit.multiplier", "test");
        }
        assert_eq!(cfg.history.len(), 20);
        let total: usize = cfg.history.iter().map(|h| h.snapshot.len()).sum();
        // flat snapshots: each ≈ base size; nested history would grow
        // super-linearly (≈ base × 2^20)
        assert!(
            total < 50 * base,
            "history snapshots nested: total {total} vs base {base}"
        );
    }

    #[test]
    fn load_sanitizes_out_of_range_fields() {
        let dir = std::env::temp_dir().join(format!("xdb-load-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config");
        let mut cfg = ConfigFile::default();
        cfg.rate_limit.min_limit = 20_000;
        cfg.rate_limit.max_limit = u32::MAX;
        cfg.global.jwt_token_lifetime_minutes = u64::MAX / 60 + 1000;
        cfg.auth.session_ttl_hours = u64::MAX;
        let bytes = serialize_with_checksum(&cfg).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let (loaded, _) = load_from_disk(&path);
        assert!(loaded.rate_limit.max_limit <= 10_000);
        assert!(loaded.rate_limit.min_limit <= loaded.rate_limit.max_limit);
        assert!(loaded.global.jwt_token_lifetime_minutes <= 60 * 24 * 30);
        assert!(loaded.auth.session_ttl_hours <= 24 * 30);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backup_rotation_chain() {
        let dir = std::env::temp_dir().join(format!("xdb-bak-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config");
        let cfg = ConfigFile::default();
        for i in 1..=7 {
            let mut c = cfg.clone();
            c.rate_limit.multiplier = i as f64;
            save_to_disk(&c, &path).unwrap();
        }
        assert!(path.exists());
        assert!(path.with_extension("bak").exists());
        for i in 2..=5 {
            assert!(
                path.with_extension(format!("bak.{i}")).exists(),
                "missing config.bak.{i}"
            );
        }
        assert!(!path.with_extension("bak.6").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn block_status() {
        let mut cfg = ConfigFile::default();
        cfg.blocked.push("provider1".into());
        assert_eq!(cfg.blocked_status("user1", "provider1"), BlockStatus::App);
        cfg.blocked.push("user1@provider1".into());
        assert_eq!(cfg.blocked_status("user1", "provider1"), BlockStatus::Name);
        assert_eq!(cfg.blocked_status("user2", "provider1"), BlockStatus::App);
        assert_eq!(cfg.blocked_status("user1", "other"), BlockStatus::None);
    }
}
