//! XavierDB — MongoDB query API server with JWT auth, granular permissions,
//! adaptive limits and a live admin dashboard.
//!
//! Routes:
//!   POST /auth                    client login -> JWT (+ cookie)
//!   /q/*                          the MongoDB proxy namespace
//!   /dashboard/                   admin dashboard (login protected)
//!   /health                       cached health document (public)

mod assets;
mod auth;
mod config;
mod dbq;
mod error;
mod health;
mod metrics;
mod perms;
mod routes_admin;
mod routes_misc;
mod routes_q;
mod state;
mod tls;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::Router;
use axum::routing::{get, post};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tracing::{error, info, warn};

use crate::perms::PermissionsFile;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// env helpers
// ---------------------------------------------------------------------------

fn env_str(key: &str, default: &str) -> String {
    // empty counts as unset (USERNAME=/PORT= must not reach the app)
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn env_path(key: &str) -> Option<PathBuf> {
    let v = std::env::var(key).ok()?;
    let v = v.trim().to_string();
    if v.is_empty() {
        return None;
    }
    Some(PathBuf::from(v))
}

/// Load .env from the working directory; create a default one when missing.
fn load_env() {
    if !std::path::Path::new(".env").exists() {
        let template = include_str!("../.env.example");
        if let Err(e) = std::fs::write(".env", template) {
            crate::state::log_line("ERROR", &format!("[env] could not create .env: {e}"));
        } else {
            crate::state::log_stdout("INFO", "[env] created .env from .env.example");
        }
    }
    if let Err(e) = dotenvy::from_path(".env") {
        crate::state::log_line("WARN", &format!("[env] dotenv error: {e}"));
    }
}

/// Bootstrap the admin password: when PASSWORD_HASH is blank or unparseable,
/// generate a strong password, hash it with Argon2id and write the hash back
/// into .env. The plaintext is printed to the terminal exactly once.
fn bootstrap_admin_password() {
    let hash = std::env::var("PASSWORD_HASH").unwrap_or_default();
    let valid = !hash.is_empty() && argon2::password_hash::PasswordHash::new(&hash).is_ok();
    if valid {
        return;
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
    let phc = match auth::hash_credential(&password) {
        Ok(h) => h,
        Err(e) => {
            crate::state::log_line("ERROR", &format!("[admin] could not hash password: {e}"));
            return;
        }
    };
    let path = ".env";
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let mut found = false;
    // NOTE: the hash contains '$' signs; dotenvy would treat them as variable
    // references, so the value MUST be single-quoted (strong quotes).
    let quoted = format!("PASSWORD_HASH='{phc}'");
    let out: String = content
        .lines()
        .map(|line| {
            if line.split_once('=').map(|(k, _)| k.trim()) == Some("PASSWORD_HASH") {
                found = true;
                quoted.clone()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let out = if found {
        out
    } else {
        format!("{out}\n{quoted}\n")
    };
    if let Err(e) = std::fs::write(path, out) {
        eprintln!("[admin] could not write .env: {e}");
        return;
    }
    // SAFETY: called from main() before the tokio runtime is built, so no
    // other thread exists yet.
    unsafe { std::env::set_var("PASSWORD_HASH", &phc) };
    crate::state::log_stdout("INFO", "================================================================");
    crate::state::log_stdout("INFO", "[admin] generated a new dashboard password (shown ONCE):");
    crate::state::log_stdout("INFO", &format!("    {password}"));
    crate::state::log_stdout(
        "INFO",
        &format!(
            "username : {}",
            std::env::var("USERNAME").unwrap_or_else(|_| "admin".into())
        ),
    );
    crate::state::log_stdout("INFO", "(stored in .env as PASSWORD_HASH — you can change it there)");
    crate::state::log_stdout("INFO", "================================================================");
}

/// Spawn a background loop task that restarts itself whenever it exits
/// (panic or early return).
fn spawn_supervised<F, Fut>(name: &'static str, factory: F)
where
    F: Fn() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        loop {
            let task = tokio::spawn(factory());
            if task.await.is_err() {
                crate::state::log_line("ERROR", &format!("[{name}] background loop panicked — restarting"));
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}

// ---------------------------------------------------------------------------
// file watchers
// ---------------------------------------------------------------------------

/// Watch a file and run `on_change` after `debounce_ms` of quiet.
fn watch_file<F>(path: PathBuf, debounce_ms: u64, mut on_change: F)
where
    F: FnMut() + Send + 'static,
{
    std::thread::spawn(move || {
        use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = match RecommendedWatcher::new(tx, Config::default()) {
            Ok(w) => w,
            Err(e) => {
                crate::state::log_line("ERROR", &format!("[watch] cannot watch {}: {e}", path.display()));
                return;
            }
        };
        if watcher.watch(&path, RecursiveMode::NonRecursive).is_err() {
            crate::state::log_line(
                "WARN",
                &format!("[watch] cannot watch {} (file may not exist yet)", path.display()),
            );
            return;
        }
        let mut pending = false;
        let mut last = std::time::Instant::now();
        loop {
            match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(Ok(ev)) => {
                    let relevant = matches!(
                        ev.kind,
                        EventKind::Modify(_)
                            | EventKind::Create(_)
                            | EventKind::Remove(_)
                            | EventKind::Any
                    );
                    if relevant {
                        pending = true;
                        last = std::time::Instant::now();
                    }
                }
                Ok(Err(e)) => crate::state::log_line("WARN", &format!("[watch] event error: {e}")),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if pending && last.elapsed() >= std::time::Duration::from_millis(debounce_ms) {
                        pending = false;
                        on_change();
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
}

fn start_watchers(state: Arc<AppState>, tls_state: Option<Arc<tls::TlsState>>) {
    // permissions file
    let st = state.clone();
    let p = st.perms_path.clone();
    watch_file(p.clone(), 500, move || {
        let bytes = std::fs::read(&p).ok();
        let last = st.last_perms_written.lock().unwrap().clone();
        if let (Some(b), Some(l)) = (&bytes, &last) {
            if b == l {
                return;
            }
        }
        match std::fs::read_to_string(&p) {
            Ok(text) => match PermissionsFile::parse(&text) {
                Ok(perms) => match perms.validate() {
                    Ok(()) => {
                        *st.perms.write().unwrap() = perms;
                        st.perms_version.fetch_add(1, Ordering::Relaxed);
                        // re-stamp with the bytes just loaded, so a later
                        // restore to the server's previous write is seen as a
                        // change again (otherwise disk and memory diverge
                        // until the next server-side write)
                        if let Some(b) = bytes {
                            *st.last_perms_written.lock().unwrap() = Some(b);
                        }
                        info!("authorized_keys.yml reloaded from disk");
                    }
                    Err(e) => error!("authorized_keys.yml invalid, keeping previous: {e}"),
                },
                Err(e) => error!("authorized_keys.yml invalid, keeping previous: {e}"),
            },
            Err(e) => error!("cannot read authorized_keys.yml: {e}"),
        }
    });

    // config file
    let st2 = state.clone();
    let p2 = st2.config_path.clone();
    watch_file(p2.clone(), 500, move || {
        let bytes = std::fs::read(&p2).ok();
        let last = st2.last_config_written.lock().unwrap().clone();
        if let (Some(b), Some(l)) = (&bytes, &last) {
            if b == l {
                return;
            }
        }
        let (cfg, err) = config::load_from_disk(&p2);
        if let Some(e) = err {
            error!("config reload failed: {e}");
            return;
        }
        crate::state::apply_log_level(&cfg.dashboard.log_level);
        *st2.config.write().unwrap() = cfg;
        st2.cfg_version.fetch_add(1, Ordering::Relaxed);
        if let Some(b) = bytes {
            *st2.last_config_written.lock().unwrap() = Some(b);
        }
        info!("config reloaded from disk");
    });

    // TLS cert/key hot reload — both files must be watched: a key-only
    // rotation would otherwise never trigger a reload
    if let Some(ts) = tls_state {
        let (cp, kp) = (ts.cert_path.clone(), ts.key_path.clone());
        for path in [cp, kp] {
            let ts2 = ts.clone();
            watch_file(path, 1000, move || match ts2.reload() {
                Ok(()) => info!(
                    "TLS certificate reloaded ({} + {})",
                    ts2.cert_path.display(),
                    ts2.key_path.display()
                ),
                Err(e) => warn!("TLS reload failed, keeping old certificate: {e}"),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// router
// ---------------------------------------------------------------------------

fn build_router(state: Arc<AppState>) -> Router {
    let q = Router::new().route(
        "/q/{db}/{coll}",
        get(routes_q::find_docs)
            .post(routes_q::insert_or_update)
            .put(routes_q::put_update)
            .patch(routes_q::patch_upsert)
            .delete(routes_q::delete_docs),
    );

    let admin = Router::new()
        .route("/dashboard/api/login", post(routes_admin::login))
        .route("/dashboard/api/logout", post(routes_admin::logout))
        .route("/dashboard/api/session", get(routes_admin::session))
        .route("/dashboard/api/metrics", get(routes_admin::metrics))
        .route("/dashboard/api/block", post(routes_admin::block))
        .route("/dashboard/api/unblock", post(routes_admin::unblock))
        .route("/dashboard/api/app_weight", post(routes_admin::app_weight))
        .route(
            "/dashboard/api/perms",
            get(routes_admin::perms_get).post(routes_admin::perms_save),
        )
        .route(
            "/dashboard/api/perms/reload",
            post(routes_admin::perms_reload),
        )
        .route(
            "/dashboard/api/config",
            get(routes_admin::config_get).post(routes_admin::config_save),
        )
        .route(
            "/dashboard/api/config/undo",
            post(routes_admin::config_undo),
        )
        .route(
            "/dashboard/api/config/redo",
            post(routes_admin::config_redo),
        )
        .route(
            "/dashboard/api/config/reload",
            post(routes_admin::config_reload),
        )
        .route(
            "/dashboard/api/config/reset",
            post(routes_admin::config_reset),
        )
        .route(
            "/dashboard/api/config/revert",
            post(routes_admin::config_revert),
        )
        .route(
            "/dashboard/api/config/export",
            get(routes_admin::config_export),
        )
        .route(
            "/dashboard/api/config/import",
            post(routes_admin::config_import),
        )
        .route("/dashboard/api/logs", get(routes_admin::logs))
        .route("/dashboard/api/databases", get(routes_admin::databases));

    Router::new()
        .merge(q)
        .merge(admin)
        .route("/ls", get(routes_q::list_visible))
        .route("/auth", post(routes_misc::auth_login))
        .route("/health", get(routes_misc::health))
        .route("/dashboard", get(assets::dashboard_index))
        .route("/dashboard/", get(assets::dashboard_index))
        .route("/dashboard/{*rest}", get(assets::dashboard_assets))
        .fallback(assets::not_found)
        .with_state(state)
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() {
    // panics go to the dashboard log ring too (the console keeps its default
    // behavior via log_line's stderr echo)
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current().name().unwrap_or("<unnamed>").to_string();
        let loc = info
            .location()
            .map(|l| format!(" at {}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().map(|s| s.clone()))
            .unwrap_or_else(|| "Box<dyn Any>".to_string());
        crate::state::log_line("ERROR", &format!("thread '{thread}' panicked{loc}:\n{payload}"));
    }));

    // one shared crypto provider for rustls + jsonwebtoken (ring is already
    // in the tree via jsonwebtoken's rust_crypto feature)
    let _ = rustls::crypto::ring::default_provider().install_default();

    load_env();
    // must run before the tokio runtime exists (it uses unsafe set_var)
    bootstrap_admin_password();

    let max_workers: usize = std::env::var("MAX_WORKERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n| *n > 0 && *n <= 512) // tokio panics above 512 workers
        .unwrap_or(4);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(max_workers)
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");
    rt.block_on(run(max_workers));
}

async fn run(max_workers: usize) {
    let host = env_str("HOST", "127.0.0.1");
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .filter(|p| *p > 0) // PORT=0 would bind an ephemeral port silently
        .unwrap_or(8000);
    let max_insert_batch: usize = std::env::var("MAX_INSERT_BATCH")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n| *n > 0) // 0/garbage would reject every batch — fall back
        .unwrap_or(crate::routes_q::MAX_INSERT_BATCH);
    let mongodb_uri = env_str("MONGODB_URI", "mongodb://localhost:27017");

    // --- config + perms ---
    let config_path = PathBuf::from("config");
    let (config, cfg_warn) = config::load_from_disk(&config_path);
    if let Some(w) = cfg_warn {
        crate::state::log_line("WARN", &format!("[config] {w}"));
    }
    let perms_path = PathBuf::from(&config.global.permission_file);
    let perms = match std::fs::read_to_string(&perms_path) {
        Ok(text) => match PermissionsFile::parse(&text) {
            Ok(p) => p,
            Err(e) => {
                crate::state::log_line("ERROR", &format!("[perms] {e} — starting with no permissions"));
                PermissionsFile::default()
            }
        },
        Err(_) => PermissionsFile::default(),
    };
    if perms.validate().is_err() {
        crate::state::log_line("WARN", "[perms] warning: authorized_keys.yml has validation problems");
    }

    // --- JWT secret ---
    let jwt_secret: [u8; 32] = match std::env::var("JWT_SECRET") {
        Ok(s) if !s.is_empty() => {
            let digest = Sha256::digest(s.as_bytes());
            let mut h = [0u8; 32];
            h.copy_from_slice(&digest);
            h
        }
        _ => {
            use rand::RngCore;
            let mut b = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut b);
            crate::state::log_line(
                "WARN",
                "[auth] JWT_SECRET not set: generated a random secret for this run; all existing tokens will be invalid after restart",
            );
            b
        }
    };

    // --- MongoDB ---
    let mongo = match mongodb::Client::with_uri_str(&mongodb_uri).await {
        Ok(c) => c,
        Err(e) => {
            crate::state::log_line("ERROR", &format!("[mongo] bad URI: {e}"));
            std::process::exit(1);
        }
    };

    // --- TLS ---
    let (tls_state, https) = match (env_path("TLS_CERT_PATH"), env_path("TLS_KEY_PATH")) {
        (Some(cert), Some(key)) => match tls::TlsState::new(cert, key) {
            Ok(ts) => (Some(Arc::new(ts)), true),
            Err(e) => {
                crate::state::log_line("WARN", &format!("[tls] TLS configured but unusable ({e}) — falling back to plain HTTP"));
                (None, false)
            }
        },
        _ => (None, false),
    };

    // --- logging (stdout + in-memory ring for the dashboard) ---
    // Verbosity comes from config dashboard.log_level ("info"|"debug") and is
    // hot-reloadable via the reload handle (dashboard save / watcher).
    let log_level = if config.dashboard.log_level == "debug" {
        tracing_subscriber::filter::LevelFilter::DEBUG
    } else {
        tracing_subscriber::filter::LevelFilter::INFO
    };
    let (log_filter, log_reload) = tracing_subscriber::reload::Layer::new(log_level);
    crate::state::set_log_level_hook(Box::new(move |level: &str| {
        let lf = if level == "debug" {
            tracing_subscriber::filter::LevelFilter::DEBUG
        } else {
            tracing_subscriber::filter::LevelFilter::INFO
        };
        let _ = log_reload.modify(|f| *f = lf);
    }));

    let state = AppState::new(
        config,
        perms,
        jwt_secret,
        https,
        mongo,
        config_path,
        perms_path,
        env_str("USERNAME", "admin"),
        max_insert_batch,
    );

    use tracing_subscriber::layer::{Layer, SubscriberExt};
    use tracing_subscriber::util::SubscriberInitExt;
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(|| LogWriter)
                .with_filter(log_filter),
        )
        .try_init()
        .ok();

    // --- background tasks (supervised: a panicking loop restarts itself —
    // a dead metrics/health loop would leave stale status forever, e.g.
    // /health serving a frozen "ok") ---
    let st = state.clone();
    spawn_supervised("metrics", move || metrics::metrics_loop(st.clone()));
    let st = state.clone();
    spawn_supervised("health", move || health::health_loop(st.clone()));
    start_watchers(state.clone(), tls_state.clone());

    // --- serve ---
    let addr = format!("{host}:{port}");
    let listener = TcpListener::bind(&addr).await.unwrap_or_else(|e| {
        crate::state::log_line("ERROR", &format!("cannot bind {addr}: {e}"));
        std::process::exit(1);
    });
    info!(
        "XavierDB listening on {addr} ({} workers, mongo={mongodb_uri}, tls={https})",
        max_workers
    );

    let app = build_router(state.clone()).into_make_service_with_connect_info::<tls::MyAddr>();

    let shutdown = async {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        {
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
            tokio::select! {
                _ = ctrl_c => {},
                _ = sigterm.recv() => {},
            }
        }
        #[cfg(not(unix))]
        {
            let _ = ctrl_c.await;
        }
        info!("shutdown requested, draining connections");
    };

    match tls_state {
        Some(ts) => {
            axum::serve(tls::TlsIncoming::new(listener, ts), app)
                .with_graceful_shutdown(shutdown)
                .await
                .expect("server error");
        }
        None => {
            axum::serve(tls::PlainIncoming { listener }, app)
                .with_graceful_shutdown(shutdown)
                .await
                .expect("server error");
        }
    }
    info!("server stopped");
}

/// tracing writer that also feeds the in-memory ring (dashboard logs page).
struct LogWriter;

impl std::io::Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let line = String::from_utf8_lossy(buf).trim_end().to_string();
        if !line.is_empty() {
            // strip ANSI color escapes (tracing adds them; the dashboard can't render them)
            let clean = strip_ansi(&line);
            state::log_push_raw(clean);
        }
        std::io::stdout().write_all(buf)?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        std::io::stdout().flush()
    }
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c2 in chars.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}
