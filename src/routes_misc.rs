//! /auth (client login -> JWT) and /health (cached, public).

use std::sync::Arc;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use crate::auth::{auth_throttled, parse_identifier, sign_jwt, verify_credential};
use crate::error::{ApiError, JsonBody};
use crate::state::AppState;
use tracing::{info, warn};

/// Truncate a client-supplied identity before logging (it is attacker input).
fn log_ident(s: &str) -> String {
    s.chars().take(100).collect()
}

// ---------------------------------------------------------------------------
// POST /auth  { "identifier": "name@app", "token": "..." }
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct AuthBody {
    pub identifier: String,
    pub token: String,
}

pub async fn auth_login(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<crate::tls::MyAddr>,
    headers: HeaderMap,
    JsonBody(body): JsonBody<AuthBody>,
) -> Result<impl IntoResponse, ApiError> {
    // Throttle keys on the client IP: the proxy header IP when
    // network.trust_proxy_headers is on (deployment behind a reverse proxy
    // on localhost), else the peer socket IP — X-Forwarded-For is
    // client-controlled, so trusting it without a proxy would let a caller
    // rotate the header to bypass the brute-force limit.
    let ip = crate::routes_q::effective_ip(&state, &headers, &addr.0);
    let from = crate::routes_q::effective_addr(&state, &headers, &addr.0);
    if let Err(e) = auth_throttled(&state, &ip) {
        warn!("login throttled: {ip}");
        return Err(e);
    }

    let (name, app) = match parse_identifier(&body.identifier) {
        Some(p) => p,
        None => {
            warn!(
                "login failed: {} from {}",
                log_ident(&body.identifier),
                from
            );
            return Err(ApiError::unauthorized());
        }
    };

    // blocked? (cheap check before the expensive hash; also makes the 403
    // independent of token correctness)
    if let Err(e) = crate::routes_q::check_block(&state, &name, &app) {
        warn!(
            "login blocked: {} from {}",
            log_ident(&body.identifier),
            from
        );
        return Err(e);
    }

    // clone the app hash and drop the perms read-lock before hashing: the
    // Argon2id verify takes seconds and must not stall permission writers
    let hash = {
        let perms = state.perms.read().unwrap();
        perms.apps.get(&app).and_then(|e| e.token_hash.clone())
    };
    // Argon2id runs on the blocking pool so the async workers stay
    // responsive; unknown apps verify against a fixed dummy hash so the
    // response time doesn't reveal whether an app_id exists.
    let token = body.token;
    let hash = hash.unwrap_or_else(|| crate::auth::DUMMY_PHC.to_string());
    let ok = tokio::task::spawn_blocking(move || verify_credential(&token, &hash))
        .await
        .unwrap_or(false);
    if !ok {
        warn!(
            "login failed: {} from {}",
            log_ident(&body.identifier),
            from
        );
        return Err(ApiError::unauthorized());
    }
    info!("login OK: {} from {}", log_ident(&body.identifier), from);

    // first sight of this name_id -> make it editable in the file
    {
        let mut perms = state.perms.write().unwrap();
        if !perms
            .apps
            .get(&app)
            .map(|e| e.names.contains_key(&name))
            .unwrap_or(false)
        {
            if let Some(e) = perms.apps.get_mut(&app) {
                e.names.insert(name.clone(), Default::default());
                let yaml = perms.to_yaml();
                if let Ok(y) = yaml {
                    let written = crate::perms::persist_perms(&state, &y);
                    match written {
                        Ok(bytes) => {
                            *state.last_perms_written.lock().unwrap() = Some(bytes);
                            state
                                .perms_version
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        Err(e) => eprintln!(
                            "[perms] could not persist auto-created name {name}@{app}: {e}"
                        ),
                    }
                }
            }
        }
    }

    let lifetime = state
        .config
        .read()
        .map(|c| c.global.jwt_token_lifetime_minutes)
        .unwrap_or(90);
    let token = sign_jwt(&state, &name, &app, lifetime)?;

    let cookie = format!(
        "xdb_token={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        lifetime * 60
    );
    let secure = if state.https { "; Secure" } else { "" };
    let cookie = format!("{cookie}{secure}");

    Ok((
        [("set-cookie", cookie)],
        Json(json!({
            "token": token,
            "token_type": "Bearer",
            "expires_in": lifetime * 60,
            "identifier": format!("{name}@{app}"),
        })),
    ))
}

// ---------------------------------------------------------------------------
// GET /health
// ---------------------------------------------------------------------------

pub async fn health(State(state): State<Arc<AppState>>) -> Result<impl IntoResponse, ApiError> {
    let doc = state.health_cache.read().unwrap().clone();
    let (doc, status) = match doc {
        Some(d) => {
            let st = d["status"].as_str().unwrap_or("unhealthy");
            let code = if st == "ok" {
                axum::http::StatusCode::OK
            } else {
                axum::http::StatusCode::SERVICE_UNAVAILABLE
            };
            (d, code)
        }
        None => (
            json!({
                "status": "starting",
                "checked_at_ms": crate::state::now_ms(),
                "next_refresh_seconds": 5,
            }),
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
        ),
    };
    Ok((status, Json(doc)))
}
