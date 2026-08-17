//! /dashboard/api/* — admin endpoints (session cookie protected).
//! Login/logout, metrics, clients/blocking, permissions editor, config editor
//! (with undo/redo/reload), logs.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::Json;
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::{check_admin_session, create_admin_session, hash_credential};
use crate::config::ConfigFile;
use crate::error::{ApiError, JsonBody};
use crate::perms::{AppEntry, PermissionsFile, Rule, effective_rules};
use crate::state::{AppState, now_ms};

// ---------------------------------------------------------------------------
// session guard
// ---------------------------------------------------------------------------

fn session_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::COOKIE)?
        .to_str()
        .ok()
        .and_then(|s| {
            s.split(';')
                .map(|p| p.trim())
                .find_map(|p| p.strip_prefix("xdb_admin="))
                .map(|t| t.to_string())
        })
}

fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<String, ApiError> {
    let token = session_cookie(headers).ok_or_else(ApiError::unauthorized)?;
    check_admin_session(state, &token)
}

// ---------------------------------------------------------------------------
// login / logout / session
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct LoginBody {
    pub username: String,
    pub password: String,
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<crate::tls::MyAddr>,
    JsonBody(body): JsonBody<LoginBody>,
) -> Result<impl IntoResponse, ApiError> {
    // peer socket IP only — see routes_misc::auth_login (X-Forwarded-For is
    // client-controlled and must not be trusted for throttling). Dashboard
    // login has its OWN throttle (server.yml admin.max_logins_per_ip_per_minute),
    // separate from the /auth throttle (config file).
    let ip = addr.0.ip().to_string();
    crate::auth::dash_throttled(&state, &ip)?;

    let password_hash = state.password_hash.clone();
    let user_ok = body.username == state.admin_user && !password_hash.is_empty();
    // run the hash on the blocking pool (Argon2id takes seconds; the async
    // workers must stay responsive) and verify against a fixed dummy hash on
    // username mismatch so timing doesn't reveal whether the username exists
    let hash = if user_ok {
        password_hash
    } else {
        crate::auth::DUMMY_PHC.to_string()
    };
    let password = body.password.clone();
    let ok = tokio::task::spawn_blocking(move || crate::auth::verify_credential(&password, &hash))
        .await
        .unwrap_or(false);
    if !user_ok || !ok {
        tracing::warn!("admin login failed: {}", body.username.chars().take(60).collect::<String>());
        return Err(ApiError::unauthorized());
    }
    tracing::info!("admin login OK: {}", body.username.chars().take(60).collect::<String>());
    let token = create_admin_session(&state, &body.username);
    // cookie lifetime follows the server-side session TTL (configurable)
    let max_age = state
        .config
        .read()
        .map(|c| c.auth.session_ttl_hours * 3600)
        .unwrap_or(86400);
    let cookie = format!(
        "xdb_admin={token}; Path=/dashboard; HttpOnly; SameSite=Strict; Max-Age={max_age}{}",
        if state.https { "; Secure" } else { "" }
    );
    Ok(([("set-cookie", cookie)], Json(json!({ "ok": true }))))
}

pub async fn logout(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(t) = session_cookie(&headers) {
        state.sessions.remove(&t);
    }
    Ok((
        [(
            "set-cookie",
            "xdb_admin=; Path=/dashboard; HttpOnly; Max-Age=0",
        )],
        Json(json!({ "ok": true })),
    ))
}

pub async fn session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let user = require_admin(&state, &headers)?;
    Ok(Json(json!({ "username": user })))
}

// ---------------------------------------------------------------------------
// metrics
// ---------------------------------------------------------------------------

/// Owned snapshot of a client's live stats (rate, sparkline history, p50).
fn snapshot_client(stats: &crate::state::ClientStats) -> (f64, Vec<f32>, f64) {
    let hist: Vec<f32> = stats
        .history
        .lock()
        .map(|h| h.iter().copied().collect())
        .unwrap_or_default();
    let p50 = stats
        .lat
        .lock()
        .map(|l| {
            let mut v: Vec<f64> = l.iter().copied().collect();
            crate::metrics::median(&mut v)
        })
        .unwrap_or(0.0);
    (stats.rate_f64(), hist, p50)
}

pub async fn metrics(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;

    let sys = state.sys.read().unwrap().clone();
    let (cfg_version, perms_version, poll_seconds, theme, smoothing, health_ttl, multiplier) = {
        let c = state.config.read().unwrap();
        (
            state.cfg_version.load(Ordering::Relaxed),
            state.perms_version.load(Ordering::Relaxed),
            c.dashboard.poll_seconds,
            c.dashboard.theme.clone(),
            c.dashboard.graph_smoothing,
            c.health.cache_ttl_seconds,
            c.rate_limit.multiplier,
        )
    };
    let health = state
        .health_cache
        .read()
        .unwrap()
        .clone()
        .unwrap_or(json!({"status":"starting"}));
    let qps = *state.qps.read().unwrap();

    // per-app + per-name tree
    let mut apps: Vec<Value> = Vec::new();
    let perms = state.perms.read().unwrap();
    let mut app_ids: Vec<String> = perms.apps.keys().cloned().collect();
    // include apps seen live but not (yet) in the file
    for e in state.clients.iter() {
        let k = e.key();
        if let Some(app) = k.strip_prefix("app:") {
            if !app_ids.contains(&app.to_string()) {
                app_ids.push(app.to_string());
            }
        }
    }
    app_ids.sort();
    for app in &app_ids {
        let blocked_app = state.config.read().unwrap().is_blocked(app);
        let limit = state.limits.get(app).map(|l| l.enforced);
        let (app_rps, app_hist, app_p50) = match state.clients.get(&format!("app:{app}")) {
            Some(stats) => snapshot_client(&stats),
            None => (0.0, vec![], 0.0),
        };

        let mut names: Vec<Value> = Vec::new();
        let mut name_ids: Vec<String> = perms
            .apps
            .get(app)
            .map(|e| e.names.keys().cloned().collect())
            .unwrap_or_default();
        for e in state.clients.iter() {
            let k = e.key();
            if let Some(rest) = k.strip_prefix("name:") {
                if let Some((n, a)) = rest.rsplit_once('@') {
                    if a == app && !name_ids.contains(&n.to_string()) {
                        name_ids.push(n.to_string());
                    }
                }
            }
        }
        name_ids.sort();
        for nid in &name_ids {
            let key = format!("name:{nid}@{app}");
            let (rps, hist, p50, total, last_seen) = match state.clients.get(&key) {
                Some(stats) => {
                    let (r, h, p) = snapshot_client(&stats);
                    (
                        r,
                        h,
                        p,
                        stats.total.load(Ordering::Relaxed),
                        stats.last_seen.load(Ordering::Relaxed),
                    )
                }
                None => (0.0, vec![], 0.0, 0, 0),
            };
            names.push(json!({
                "name": nid,
                "id": format!("{nid}@{app}"),
                "blocked": state.config.read().unwrap().is_blocked(&format!("{nid}@{app}")),
                "rps": rps,
                "p50_ms": p50,
                "total_requests": total,
                "last_seen_ms": last_seen,
                "rps_history": hist,
            }));
        }
        let breakdown = state.limits.get(app).map(|l| {
            json!({
                "internal": l.internal,
                "enforced": l.enforced,
                "lat_err": l.lat_err,
                "pressure": l.pressure,
                "shrink": l.shrink,
                "p50_ms": l.p50_ms,
                "rate": l.rate,
                "updated_ms": l.updated_ms,
            })
        });
        apps.push(json!({
            "app": app,
            "blocked": blocked_app,
            "weight": state.config.read().unwrap().rate_limit.weights.get(app).copied().unwrap_or(1.0),
            "rps": app_rps,
            "p50_ms": app_p50,
            "limit": limit,
            "breakdown": breakdown,
            "rps_history": app_hist,
            "names": names,
        }));
    }
    drop(perms);

    // cursors
    let mut curs: Vec<Value> = state
        .cursors
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "db": c.db,
                "coll": c.coll,
                "created_ms": c.created_ms,
                "last_used_ms": c.last_used_ms.load(Ordering::Relaxed),
                "uses": c.uses.load(Ordering::Relaxed),
            })
        })
        .collect();
    curs.sort_by_key(|c| -c["last_used_ms"].as_i64().unwrap_or(0));
    curs.truncate(30);

    Ok(Json(json!({
        "ts": now_ms(),
        "config": {
            "poll_seconds": poll_seconds,
            "theme": theme,
            "graph_smoothing": smoothing,
            "cfg_version": cfg_version,
            "perms_version": perms_version,
            "health_ttl_seconds": health_ttl,
            "multiplier": multiplier,
        },
        "system": sys,
        "qps": qps,
        "health": health,
        "apps": apps,
        "cursors": { "count": state.cursors.len(), "list": curs },
    })))
}

// ---------------------------------------------------------------------------
// block / unblock
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct BlockBody {
    pub id: String, // "name@app" or bare "app"
}

fn config_change(
    state: &AppState,
    desc: &str,
    path: &str,
    f: impl FnOnce(&mut ConfigFile),
) -> Result<(), ApiError> {
    let mut cfg = state.config.write().unwrap().clone();
    let new = cfg.clone();
    cfg.apply(new, desc, path, "dashboard");
    f(&mut cfg);
    save_config(state, &cfg)?;
    Ok(())
}

pub async fn block(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    JsonBody(body): JsonBody<BlockBody>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let id = body.id.trim().to_string();
    if id.is_empty() || id.len() > 130 {
        return Err(ApiError::bad_request("invalid id"));
    }
    config_change(&state, &format!("block {id}"), "blocked", |cfg| {
        if !cfg.blocked.contains(&id) {
            cfg.blocked.push(id.clone());
            cfg.blocked.sort();
        }
    })?;
    Ok(Json(json!({ "ok": true, "id": id })))
}

pub async fn unblock(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    JsonBody(body): JsonBody<BlockBody>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    config_change(&state, &format!("unblock {}", body.id), "blocked", |cfg| {
        cfg.blocked.retain(|b| b != &body.id);
    })?;
    Ok(Json(json!({ "ok": true, "id": body.id })))
}

#[derive(Deserialize)]
pub struct WeightBody {
    pub id: String,  // app_id
    pub weight: f64, // 0.1 ..= 10.0
}

/// Set the adaptive-limit weight of one app_id (enforced = limit * multiplier * weight).
pub async fn app_weight(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    JsonBody(body): JsonBody<WeightBody>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let id = body.id.trim().to_string();
    if id.is_empty() || id.len() > 130 {
        return Err(ApiError::bad_request("invalid id"));
    }
    if !(0.1..=10.0).contains(&body.weight) {
        return Err(ApiError::bad_request("weight must be between 0.1 and 10.0"));
    }
    let w = (body.weight * 10.0).round() / 10.0; // snap to 0.1 steps
    config_change(
        &state,
        &format!("weight {id} = {w:.1}"),
        &format!("rate_limit.weights.{id}"),
        |cfg| {
            cfg.rate_limit.weights.insert(id.clone(), w);
        },
    )?;
    Ok(Json(json!({ "ok": true, "id": body.id, "weight": w })))
}

/// persist config + bump version. Must NOT hold the config write lock.
fn save_config(state: &AppState, cfg: &ConfigFile) -> Result<(), ApiError> {
    let bytes = crate::config::save_to_disk(cfg, &state.config_path)
        .map_err(|e| ApiError::internal(format!("config save failed: {e}")))?;
    *state.last_config_written.lock().unwrap() = Some(bytes);
    crate::state::apply_log_level(&cfg.dashboard.log_level);
    *state.config.write().unwrap() = cfg.clone();
    state.cfg_version.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

// ---------------------------------------------------------------------------
// permissions (view / edit / reload / set-token)
// ---------------------------------------------------------------------------

pub async fn perms_get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let perms = state.perms.read().unwrap();
    let mut apps = Vec::new();
    for (app, entry) in &perms.apps {
        let mut names = Vec::new();
        for (name, nentry) in &entry.names {
            names.push(json!({
                "name": name,
                "allow": nentry.allow,
                "deny": nentry.deny,
                "effective": effective_rules(&perms, app, Some(name)),
            }));
        }
        apps.push(json!({
            "app": app,
            "token_set": entry.token_hash.is_some(),
            "allow": entry.allow,
            "deny": entry.deny,
            "effective": effective_rules(&perms, app, None),
            "names": names,
        }));
    }
    Ok(Json(json!({
        "version": state.perms_version.load(Ordering::Relaxed),
        "apps": apps,
    })))
}

#[derive(Deserialize)]
pub struct PermsSaveBody {
    pub apps: Vec<PermsAppIn>,
}

#[derive(Deserialize)]
pub struct PermsAppIn {
    pub app: String,
    #[serde(default)]
    pub allow: Vec<Rule>,
    #[serde(default)]
    pub deny: Vec<Rule>,
    #[serde(default)]
    pub names: Vec<PermsNameIn>,
    #[serde(default)]
    pub delete: bool,
    /// optional: set a new shared credential for this app
    #[serde(default)]
    pub set_token: Option<String>,
}

#[derive(Deserialize)]
pub struct PermsNameIn {
    pub name: String,
    #[serde(default)]
    pub allow: Vec<Rule>,
    #[serde(default)]
    pub deny: Vec<Rule>,
    #[serde(default)]
    pub delete: bool,
}

pub async fn perms_save(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    JsonBody(body): JsonBody<PermsSaveBody>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;

    let mut perms = state.perms.read().unwrap().clone();
    for a in &body.apps {
        if a.delete {
            perms.apps.remove(&a.app);
            continue;
        }
        let entry = perms
            .apps
            .entry(a.app.clone())
            .or_insert_with(AppEntry::default);
        entry.allow = a.allow.clone();
        entry.deny = a.deny.clone();
        if let Some(t) = &a.set_token {
            if t.len() < 8 {
                return Err(ApiError::bad_request("token too short (min 8 characters)"));
            }
            entry.token_hash = Some(hash_credential(t).map_err(|e| ApiError::internal(e))?);
        }
        for n in &a.names {
            if n.delete {
                entry.names.remove(&n.name);
            } else {
                let ne = entry.names.entry(n.name.clone()).or_default();
                ne.allow = n.allow.clone();
                ne.deny = n.deny.clone();
            }
        }
    }
    perms
        .validate()
        .map_err(|e| ApiError::bad_request(format!("invalid permissions: {e}")))?;

    let yaml = perms.to_yaml().map_err(|e| ApiError::internal(e))?;
    let bytes = crate::perms::persist_perms(&state, &yaml).map_err(|e| ApiError::internal(e))?;
    *state.last_perms_written.lock().unwrap() = Some(bytes);
    *state.perms.write().unwrap() = perms;
    state.perms_version.fetch_add(1, Ordering::Relaxed);
    Ok(Json(json!({ "ok": true })))
}

pub async fn perms_reload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let text = std::fs::read_to_string(&state.perms_path).map_err(|e| {
        ApiError::internal(format!("cannot read {}: {e}", state.perms_path.display()))
    })?;
    let perms = PermissionsFile::parse(&text).map_err(|e| ApiError::bad_request(e))?;
    perms
        .validate()
        .map_err(|e| ApiError::bad_request(format!("invalid permissions: {e}")))?;
    *state.perms.write().unwrap() = perms;
    state.perms_version.fetch_add(1, Ordering::Relaxed);
    Ok(Json(json!({ "ok": true, "reloaded": true })))
}

// ---------------------------------------------------------------------------
// config (view / save / undo / redo / reload / reset / export)
// ---------------------------------------------------------------------------

fn config_view(state: &AppState) -> Value {
    let c = state.config.read().unwrap().clone();
    let history = c
        .history_meta()
        .into_iter()
        .map(|(ts, desc, path, by)| json!({ "ts": ts, "desc": desc, "path": path, "by": by }))
        .collect::<Vec<_>>();
    json!({
        "version": state.cfg_version.load(Ordering::Relaxed),
        "config": c,
        "history": history,
        "redo_available": !c.redo.is_empty(),
        "undo_available": !c.history.is_empty(),
    })
}

pub async fn config_get(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    Ok(Json(config_view(&state)))
}

#[derive(Deserialize)]
pub struct ConfigSaveBody {
    pub config: ConfigFile,
}

pub async fn config_save(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    JsonBody(body): JsonBody<ConfigSaveBody>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let mut new = body.config;
    sanitize_config(&mut new);
    let mut cfg = state.config.read().unwrap().clone();
    cfg.apply(new, "config edited from dashboard", "config", "dashboard");
    save_config(&state, &cfg)?;
    Ok(Json(config_view(&state)))
}

/// Clamp all user-supplied config fields to safe ranges (ConfigFile::sanitize).
fn sanitize_config(c: &mut ConfigFile) {
    c.sanitize();
}

pub async fn config_undo(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let mut cfg = state.config.read().unwrap().clone();
    let did = cfg.undo().is_some();
    if did {
        save_config(&state, &cfg)?;
    }
    Ok(Json(json!({ "ok": did })))
}

pub async fn config_redo(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let mut cfg = state.config.read().unwrap().clone();
    let did = cfg.redo().is_some();
    if did {
        save_config(&state, &cfg)?;
    }
    Ok(Json(json!({ "ok": did })))
}

pub async fn config_reload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let (cfg, err) = crate::config::load_from_disk(&state.config_path);
    crate::state::apply_log_level(&cfg.dashboard.log_level);
    *state.config.write().unwrap() = cfg;
    state.cfg_version.fetch_add(1, Ordering::Relaxed);
    Ok(Json(json!({ "ok": true, "warning": err })))
}

pub async fn config_revert(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    JsonBody(body): JsonBody<serde_json::Value>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let idx = body["index"].as_u64().unwrap_or(u64::MAX) as usize;
    let cfg = state.config.read().unwrap().clone();
    if idx >= cfg.history.len() {
        return Err(ApiError::bad_request("invalid history index"));
    }
    // The dashboard sends the display position (0 = newest entry); history is
    // stored oldest-first, so translate before indexing.
    let stored = cfg.history.len() - 1 - idx;
    let snapshot = cfg.history[stored].snapshot.clone();
    let mut prev = crate::config::decode(&snapshot).map_err(|e| ApiError::internal(e))?;
    sanitize_config(&mut prev);
    // snapshots are flat (no history inside): rebuild the entry list explicitly
    prev.history = cfg.history[..stored].to_vec();
    prev.redo.clear();
    prev.last_modified = crate::state::now_ms() / 1000;
    save_config(&state, &prev)?;
    Ok(Json(config_view(&state)))
}

pub async fn config_reset(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let mut cfg = state.config.read().unwrap().clone();
    cfg.apply(
        ConfigFile::default(),
        "reset to defaults",
        "config",
        "dashboard",
    );
    save_config(&state, &cfg)?;
    Ok(Json(config_view(&state)))
}

pub async fn config_export(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    require_admin(&state, &headers)?;
    let c = state.config.read().unwrap();
    let body = serde_json::to_string_pretty(&*c).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok((
        StatusCode::OK,
        [
            ("content-type", "application/json"),
            (
                "content-disposition",
                "attachment; filename=\"config.json\"",
            ),
        ],
        body,
    ))
}

#[derive(Deserialize)]
pub struct ConfigImportBody {
    pub config: ConfigFile,
}

pub async fn config_import(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    JsonBody(body): JsonBody<ConfigImportBody>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let mut cfg = state.config.read().unwrap().clone();
    let mut new = body.config;
    sanitize_config(&mut new);
    cfg.apply(new, "imported config", "config", "dashboard");
    save_config(&state, &cfg)?;
    Ok(Json(config_view(&state)))
}

// ---------------------------------------------------------------------------
// logs
// ---------------------------------------------------------------------------

/// Enumerate actual databases (+ their collections) in the MongoDB instance
/// (for the dashboard permission editor). Admin-only; degraded to an empty
/// list when MongoDB is unreachable.
pub async fn databases(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let names = match crate::dbq::list_databases(&state).await {
        Ok(n) => n,
        Err(_) => return Ok(Json(json!({ "databases": [], "unavailable": true }))),
    };
    let mut dbs = Vec::new();
    for n in &names {
        let colls = crate::dbq::list_collections(&state, n)
            .await
            .unwrap_or_default();
        dbs.push(json!({ "name": n, "collections": colls }));
    }
    Ok(Json(json!({ "databases": dbs, "unavailable": false })))
}

#[derive(Deserialize)]
pub struct LogsQuery {
    /// max entries (0 = all); the dashboard pages with 300 + before=<seq>
    pub limit: Option<usize>,
    /// only entries with seq < before (load-older paging)
    pub before: Option<u64>,
}

pub async fn logs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<LogsQuery>,
) -> Result<Json<Value>, ApiError> {
    require_admin(&state, &headers)?;
    let limit = q.limit.unwrap_or(0).min(10_000);
    let (entries, total, apps, names, loggers) = crate::state::log_snapshot(limit, q.before);
    let (files, size_mb, path) = crate::state::log_retention();
    Ok(Json(json!({
        "lines": entries
            .iter()
            .map(|e| json!({
                "seq": e.seq,
                "raw": e.raw,
                "level": e.level,
                "logger": e.logger,
                "app": e.app,
                "name": e.name,
            }))
            .collect::<Vec<_>>(),
        "total": total,
        "apps": apps,
        "names": names
            .iter()
            .map(|(a, n)| json!({ "app": a, "name": n }))
            .collect::<Vec<_>>(),
        "loggers": loggers,
        "retention": { "files": files, "size_mb": size_mb, "path": path },
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> crate::config::ConfigFile {
        crate::config::ConfigFile::default()
    }

    #[test]
    fn sanitize_keeps_min_max_invariant() {
        let mut c = cfg();
        c.rate_limit.min_limit = 20_000;
        c.rate_limit.max_limit = 100;
        sanitize_config(&mut c);
        assert_eq!(c.rate_limit.min_limit, 10_000);
        assert_eq!(c.rate_limit.max_limit, 10_000);
        assert!(c.rate_limit.min_limit <= c.rate_limit.max_limit);
    }

    #[test]
    fn sanitize_clamps_weights() {
        let mut c = cfg();
        c.rate_limit.weights.insert("app1".into(), -5.0);
        c.rate_limit.weights.insert("app2".into(), 100.0);
        c.rate_limit.weights.insert("app3".into(), 2.0);
        sanitize_config(&mut c);
        assert_eq!(c.rate_limit.weights["app1"], 0.1);
        assert_eq!(c.rate_limit.weights["app2"], 10.0);
        assert_eq!(c.rate_limit.weights["app3"], 2.0);
    }

    #[test]
    fn sanitize_theme_and_ranges() {
        let mut c = cfg();
        c.dashboard.theme = "neon".into();
        c.dashboard.poll_seconds = 0.05;
        c.dashboard.log_level = "verbose".into();
        c.health.cache_ttl_seconds = 999_999;
        c.auth.session_ttl_hours = 0;
        sanitize_config(&mut c);
        assert_eq!(c.dashboard.theme, "system");
        assert_eq!(c.dashboard.poll_seconds, 0.1);
        assert_eq!(c.dashboard.log_level, "info");
        assert_eq!(c.health.cache_ttl_seconds, 3600);
        assert_eq!(c.auth.session_ttl_hours, 1);
        c.dashboard.poll_seconds = 9999.0;
        sanitize_config(&mut c);
        assert_eq!(c.dashboard.poll_seconds, 3600.0);
    }
}
