//! Startup-only server settings (`server.yml`), loaded once at boot.
//!
//! Replaces the old `.env`: everything that used to live in `.env` now lives
//! in `server.yml` (gitignored; `server.yml.example` is the tracked template,
//! embedded into the binary for first-boot bootstrap). `.env` is left with
//! only `UID`/`GID`, consumed exclusively by Docker-compose interpolation
//! (`user: "$UID:$GID"`) — the app never reads `.env` anymore.
//!
//! Precedence (deliberate, so Docker compose can inject container values):
//!   OS environment variable (set + non-empty)  >  server.yml  >  baked-in default.
//! `HOST`/`MONGODB_URI` are NOT in the file by default — bare metal falls
//! back to the code defaults (127.0.0.1 / mongodb://localhost:27017) and
//! compose provides them as env vars. EXCEPTION: `admin.username` and
//! `admin.password_hash` ALWAYS come from the file — Windows always sets
//! `USERNAME` to the login name and would otherwise silently override the
//! dashboard user (this is why the old dotenv load used from_path_override).
//! No hot reload: restart to apply.

use std::path::Path;

use serde::{Deserialize, Serialize};

pub const SETTINGS_FILE: &str = "server.yml";

// ---------------------------------------------------------------------------
// structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ServerSettings {
    pub tls: TlsSettings,
    pub network: NetworkSettings,
    pub runtime: RuntimeSettings,
    pub log: LogSettings,
    pub admin: AdminSettings,
    pub auth: AuthSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TlsSettings {
    /// PEM certificate path; empty + empty key = plain HTTP.
    pub cert_path: String,
    /// PEM private key path.
    pub key_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NetworkSettings {
    pub host: String,
    /// 0 is rejected (would bind an ephemeral port silently).
    pub port: u16,
    pub mongodb_uri: String,
    /// Trust X-Real-IP / X-Forwarded-For from the reverse proxy in front.
    /// Enable ONLY when the server is not directly reachable (e.g. the port
    /// is bound to 127.0.0.1 and nginx forwards to it): then the proxy-set
    /// headers are the real client IPs, used for the login throttles and
    /// request log lines. When directly reachable, keep this OFF — the
    /// headers are client-controlled and would allow throttle evasion.
    pub trust_proxy_headers: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RuntimeSettings {
    /// Tokio worker threads (tokio panics above 512).
    pub max_workers: usize,
    /// Max documents per insert batch (POST /q array `data`).
    pub max_insert_batch: usize,
    /// Server-side deadline for MongoDB find queries (GET /q), in
    /// milliseconds. A runaway query (e.g. a multiplanner blowup on an
    /// unindexed sort over a huge collection) fails with a clean 504
    /// TIMEOUT instead of hanging until the HTTP caller gives up and
    /// severs the connection mid-operation. 0 disables; nonzero values
    /// are clamped 100..=3_600_000.
    pub find_timeout_ms: u64,
    /// Keyset-pagination type-bracket mode, "all" | "id-only" | "off"
    /// (see dbq::KeysetTypeBrackets). Startup-only. Invalid values fall
    /// back to "all" with a WARN.
    pub keyset_type_brackets: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LogSettings {
    /// Rotating log files on disk, 1..=10.
    pub files: usize,
    /// Size per file in MB, 1..=20.
    pub size_mb: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AdminSettings {
    pub username: String,
    /// Argon2id PHC hash; blank/unparseable -> generated once at boot and
    /// written back into server.yml.
    pub password_hash: String,
    /// Dashboard login throttle per IP per minute, 1..=10_000. The /auth
    /// throttle is separate (config file, auth.max_per_minute_per_ip).
    pub max_logins_per_ip_per_minute: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AuthSettings {
    /// JWT signing secret; empty -> random per start (all tokens invalidated).
    pub jwt_secret: String,
}

impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            tls: TlsSettings::default(),
            network: NetworkSettings::default(),
            runtime: RuntimeSettings::default(),
            log: LogSettings::default(),
            admin: AdminSettings::default(),
            auth: AuthSettings::default(),
        }
    }
}

impl Default for TlsSettings {
    fn default() -> Self {
        Self {
            cert_path: String::new(),
            key_path: String::new(),
        }
    }
}

impl Default for NetworkSettings {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8000,
            mongodb_uri: "mongodb://localhost:27017".to_string(),
            trust_proxy_headers: false,
        }
    }
}

impl Default for RuntimeSettings {
    fn default() -> Self {
        Self {
            max_workers: 4,
            max_insert_batch: crate::routes_q::MAX_INSERT_BATCH,
            find_timeout_ms: 10_000,
            keyset_type_brackets: "all".to_string(),
        }
    }
}

impl Default for LogSettings {
    fn default() -> Self {
        Self {
            files: 5,
            size_mb: 10,
        }
    }
}

impl Default for AdminSettings {
    fn default() -> Self {
        Self {
            username: "admin".to_string(),
            password_hash: String::new(),
            max_logins_per_ip_per_minute: 5,
        }
    }
}

impl Default for AuthSettings {
    fn default() -> Self {
        Self {
            jwt_secret: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// loading
// ---------------------------------------------------------------------------

/// Non-empty OS env var (empty counts as unset — `PORT=`/`HOST=` must not
/// reach the app). Also used outside settings (e.g. `DISK_PATH` in metrics).
pub(crate) fn env_str(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Load `server.yml`, creating it from the embedded template when missing.
/// Then apply env overrides and clamps. Never fails: on a broken/missing
/// file it warns and falls back to defaults (+ env overrides).
pub fn load() -> ServerSettings {
    if !Path::new(SETTINGS_FILE).exists() {
        let template = include_str!("../server.yml.example");
        if let Err(e) = std::fs::write(SETTINGS_FILE, template) {
            crate::state::log_line(
                "ERROR",
                &format!("[settings] could not create {SETTINGS_FILE}: {e}"),
            );
        } else {
            crate::state::log_stdout(
                "INFO",
                &format!("[settings] created {SETTINGS_FILE} from server.yml.example"),
            );
        }
    }

    let mut s = match std::fs::read_to_string(SETTINGS_FILE) {
        Ok(text) => match serde_yaml::from_str::<ServerSettings>(&text) {
            Ok(s) => s,
            Err(e) => {
                crate::state::log_line(
                    "WARN",
                    &format!(
                        "[settings] {SETTINGS_FILE} invalid ({e}) — using defaults + env overrides"
                    ),
                );
                ServerSettings::default()
            }
        },
        Err(e) => {
            crate::state::log_line(
                "WARN",
                &format!(
                    "[settings] cannot read {SETTINGS_FILE} ({e}) — using defaults + env overrides"
                ),
            );
            ServerSettings::default()
        }
    };

    // env overrides: OS env wins over the file (so Docker compose
    // `environment:` can inject container-appropriate values; bare metal
    // never sets these and uses the file/default).
    if let Some(v) = env_str("HOST") {
        s.network.host = v;
    }
    if let Some(v) = env_str("PORT") {
        if let Ok(p) = v.parse::<u16>() {
            s.network.port = p;
        }
    }
    if let Some(v) = env_str("MONGODB_URI") {
        s.network.mongodb_uri = v;
    }
    if let Some(v) = env_str("TRUST_PROXY_HEADERS") {
        s.network.trust_proxy_headers =
            matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes");
    }
    if let Some(v) = env_str("TLS_CERT_PATH") {
        s.tls.cert_path = v;
    }
    if let Some(v) = env_str("TLS_KEY_PATH") {
        s.tls.key_path = v;
    }
    if let Some(v) = env_str("MAX_WORKERS") {
        if let Ok(n) = v.parse::<usize>() {
            s.runtime.max_workers = n;
        }
    }
    if let Some(v) = env_str("MAX_INSERT_BATCH") {
        if let Ok(n) = v.parse::<usize>() {
            s.runtime.max_insert_batch = n;
        }
    }
    if let Some(v) = env_str("FIND_TIMEOUT_MS") {
        if let Ok(n) = v.parse::<u64>() {
            s.runtime.find_timeout_ms = n;
        }
    }
    if let Some(v) = env_str("KEYSET_TYPE_BRACKETS") {
        s.runtime.keyset_type_brackets = v;
    }
    if let Some(v) = env_str("LOG_FILES") {
        if let Ok(n) = v.parse::<usize>() {
            s.log.files = n;
        }
    }
    if let Some(v) = env_str("LOG_SIZE_MB") {
        if let Ok(n) = v.parse::<usize>() {
            s.log.size_mb = n;
        }
    }
    // NOTE: NO env override for USERNAME / PASSWORD_HASH — admin.username and
    // admin.password_hash always come from server.yml (Windows always sets
    // USERNAME; the hash is the credential). See the module doc.
    if let Some(v) = env_str("MAX_LOGINS_PER_IP_PER_MINUTE") {
        if let Ok(n) = v.parse::<u32>() {
            s.admin.max_logins_per_ip_per_minute = n;
        }
    }
    if let Some(v) = env_str("JWT_SECRET") {
        s.auth.jwt_secret = v;
    }

    s.clamp();
    s
}

impl ServerSettings {
    /// Clamp every field to its safe range (mirrors the old env parsing).
    fn clamp(&mut self) {
        if self.network.host.trim().is_empty() {
            self.network.host = "127.0.0.1".to_string();
        }
        if self.network.port == 0 {
            self.network.port = 8000;
        }
        if self.network.mongodb_uri.trim().is_empty() {
            self.network.mongodb_uri = "mongodb://localhost:27017".to_string();
        }
        if self.runtime.max_workers == 0 || self.runtime.max_workers > 512 {
            self.runtime.max_workers = 4;
        }
        if self.runtime.max_insert_batch == 0 {
            self.runtime.max_insert_batch = crate::routes_q::MAX_INSERT_BATCH;
        }
        // 0 = disabled; anything else must be a sane deadline
        if self.runtime.find_timeout_ms != 0 {
            self.runtime.find_timeout_ms = self.runtime.find_timeout_ms.clamp(100, 3_600_000);
        }
        if crate::dbq::KeysetTypeBrackets::parse(&self.runtime.keyset_type_brackets).is_none() {
            crate::state::log_line(
                "WARN",
                &format!(
                    "[settings] runtime.keyset_type_brackets must be all|id-only|off (got {:?}) — using \"all\"",
                    self.runtime.keyset_type_brackets
                ),
            );
            self.runtime.keyset_type_brackets = "all".to_string();
        }
        self.log.files = self.log.files.clamp(1, 10);
        self.log.size_mb = self.log.size_mb.clamp(1, 20);
        if self.admin.username.trim().is_empty() {
            self.admin.username = "admin".to_string();
        }
        self.admin.max_logins_per_ip_per_minute =
            self.admin.max_logins_per_ip_per_minute.clamp(1, 10_000);
    }

    /// Generate a strong password when `password_hash` is blank or unparseable,
    /// write the Argon2id hash back into `server.yml`, and return the plaintext
    /// (printed to the terminal exactly once). Returns `None` when the existing
    /// hash is valid or writing fails.
    pub fn bootstrap_admin_password(&mut self) -> Option<String> {
        let hash = &self.admin.password_hash;
        let valid = !hash.is_empty() && argon2::password_hash::PasswordHash::new(hash).is_ok();
        if valid {
            return None;
        }
        use rand::RngCore;
        const ALPHABET: &[u8] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_.~!@#$%^&*";
        let mut bytes = [0u8; 64];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let password: String = bytes
            .iter()
            .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
            .collect();
        let phc = match crate::auth::hash_credential(&password) {
            Ok(h) => h,
            Err(e) => {
                crate::state::log_line("ERROR", &format!("[admin] could not hash password: {e}"));
                return None;
            }
        };
        if let Err(e) = update_password_hash(SETTINGS_FILE.as_ref(), &phc) {
            crate::state::log_line(
                "ERROR",
                &format!("[admin] could not write {SETTINGS_FILE}: {e}"),
            );
            return None;
        }
        self.admin.password_hash = phc;
        Some(password)
    }
}

/// Rewrite a settings file, replacing the `admin.password_hash` line with the
/// given (unquoted — `$` is not special in YAML) value. Preserves the
/// line's indentation and everything else (comments, formatting); a missing
/// key is inserted as the first field of the `admin:` block.
fn update_password_hash(path: &Path, phc: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let mut found = false;
    for line in &mut lines {
        let t = line.trim_start();
        if t.starts_with("password_hash:") && !t.starts_with('#') {
            let ws: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            *line = format!("{ws}password_hash: \"{phc}\"");
            found = true;
            break;
        }
    }
    if !found {
        let idx = lines.iter().position(|l| l.trim() == "admin:").unwrap_or(0);
        lines.insert(idx + 1, format!("  password_hash: \"{phc}\""));
    }
    std::fs::write(path, lines.join("\n")).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_safe() {
        let mut s = ServerSettings::default();
        s.network.port = 0;
        s.runtime.max_workers = 9999;
        s.log.files = 0;
        s.log.size_mb = 999;
        s.admin.max_logins_per_ip_per_minute = 0;
        s.clamp();
        assert_eq!(s.network.port, 8000);
        assert_eq!(s.runtime.max_workers, 4);
        assert_eq!(s.log.files, 1);
        assert_eq!(s.log.size_mb, 20);
        assert_eq!(s.admin.max_logins_per_ip_per_minute, 1);
    }

    #[test]
    fn find_timeout_clamp() {
        let mut s = ServerSettings::default();
        assert_eq!(s.runtime.find_timeout_ms, 10_000);
        s.runtime.find_timeout_ms = 5;
        s.clamp();
        assert_eq!(s.runtime.find_timeout_ms, 100); // floor
        s.runtime.find_timeout_ms = 9_999_999;
        s.clamp();
        assert_eq!(s.runtime.find_timeout_ms, 3_600_000); // ceiling
        s.runtime.find_timeout_ms = 0;
        s.clamp();
        assert_eq!(s.runtime.find_timeout_ms, 0); // 0 = disabled, not floored
    }

    #[test]
    fn keyset_type_brackets_validation() {
        let mut s = ServerSettings::default();
        assert_eq!(s.runtime.keyset_type_brackets, "all");
        s.runtime.keyset_type_brackets = "id-only".into();
        s.clamp();
        assert_eq!(s.runtime.keyset_type_brackets, "id-only");
        s.runtime.keyset_type_brackets = "off".into();
        s.clamp();
        assert_eq!(s.runtime.keyset_type_brackets, "off");
        s.runtime.keyset_type_brackets = "bogus".into();
        s.clamp();
        assert_eq!(s.runtime.keyset_type_brackets, "all"); // fallback
        // and it round-trips through YAML
        let s: ServerSettings =
            serde_yaml::from_str("runtime:\n  keyset_type_brackets: id-only\n").unwrap();
        assert_eq!(s.runtime.keyset_type_brackets, "id-only");
        assert_eq!(s.runtime.find_timeout_ms, 10_000); // rest defaults intact
    }

    #[test]
    fn yaml_roundtrip_missing_fields_default() {
        let yaml = "network:\n  host: 0.0.0.0\n";
        let s: ServerSettings = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(s.network.host, "0.0.0.0");
        assert_eq!(s.network.port, 8000); // from Default
        assert_eq!(s.admin.username, "admin");
        assert_eq!(s.log.files, 5);
        assert_eq!(s.runtime.find_timeout_ms, 10_000); // from Default
    }

    #[test]
    fn password_hash_phc_roundtrip() {
        // `$` must survive a YAML round trip unquoted (it did in .env only
        // when single-quoted; YAML treats it as a plain scalar).
        let s: ServerSettings = serde_yaml::from_str(
            "admin:\n  password_hash: $argon2id$v=19$m=65536,t=3,p=4$salt$hash\n",
        )
        .unwrap();
        assert_eq!(
            s.admin.password_hash,
            "$argon2id$v=19$m=65536,t=3,p=4$salt$hash"
        );
    }

    #[test]
    fn password_hash_write_preserves_indentation() {
        // Regression: the write-back must keep `password_hash` inside the
        // `admin:` block (2-space indent) — a top-level key would be ignored
        // on the next boot, regenerating the password every start.
        let dir = std::env::temp_dir().join(format!("xdb-settings-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("server.yml");
        let template = "admin:\n  # comment stays\n  username: \"admin\"\n  password_hash: \"\"\n  max_logins_per_ip_per_minute: 5\n";
        std::fs::write(&path, template).unwrap();
        update_password_hash(&path, "$argon2id$v=19$m=65536,t=3,p=4$salt$hash").unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(
            out.contains("  password_hash: \"$argon2id$"),
            "hash must stay indented inside admin:\n{out}"
        );
        assert!(out.contains("# comment stays"));
        // and it round-trips back into the struct
        let s: ServerSettings = serde_yaml::from_str(&out).unwrap();
        assert_eq!(
            s.admin.password_hash,
            "$argon2id$v=19$m=65536,t=3,p=4$salt$hash"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn password_hash_write_inserts_missing_key() {
        let dir = std::env::temp_dir().join(format!("xdb-settings2-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("server.yml");
        std::fs::write(&path, "admin:\n  username: \"admin\"\n").unwrap();
        update_password_hash(&path, "$argon2id$newhash").unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        let s: ServerSettings = serde_yaml::from_str(&out).unwrap();
        assert_eq!(s.admin.password_hash, "$argon2id$newhash");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
