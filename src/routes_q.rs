//! The /q namespace: MongoDB proxy with JWT auth, granular permissions,
//! adaptive limits and cursor pagination.
//!
//! Routes:
//!   GET    /ls                   top-level: list databases the caller may GET (?db=X → its collections)
//!   GET    /q/{db}/{coll}       find (filter, limit, sort, cursor)
//!   POST   /q/{db}/{coll}       insert (no filter: object → insert_one, array → insert_many)
//!                                or update-many ($set)
//!   PUT    /q/{db}/{coll}       update-many ($set), 404 when nothing matched
//!   PATCH  /q/{db}/{coll}       upsert, 201 when a document was inserted;
//!                                array `data` (no filter) = upsert-many, 200
//!   DELETE /q/{db}/{coll}       delete-many, 404 when nothing deleted

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use axum::Json;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::HeaderMap;
use bson::Document;
use bson::doc;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::{Claims, valid_path_segment, verify_jwt};
use crate::config::BlockStatus;
use crate::dbq::{self, Cursor};
use crate::error::{ApiError, ApiErrorKind, JsonBody, QueryBody};
use crate::state::{AppState, ClientStats};

/// Default max documents per batch (POST /q with array `data`, PATCH /q
/// upsert-many) when the MAX_INSERT_BATCH env var is unset or invalid.
/// Bounds the write work a single request can trigger; also caps the
/// adaptive-limit weight that counts into per-client rate accounting.
pub const MAX_INSERT_BATCH: usize = 1000;

// ---------------------------------------------------------------------------
// auth extraction
// ---------------------------------------------------------------------------

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(s) = v.to_str() {
            if let Some(rest) = s.strip_prefix("Bearer ") {
                return Some(rest.trim().to_string());
            }
        }
    }
    // cookie fallback: xdb_token=...
    if let Some(v) = headers.get(axum::http::header::COOKIE) {
        if let Ok(s) = v.to_str() {
            for part in s.split(';') {
                let part = part.trim();
                if let Some(t) = part.strip_prefix("xdb_token=") {
                    return Some(t.trim().to_string());
                }
            }
        }
    }
    None
}

/// Authenticate (JWT) + block check. Returns the claims.
pub fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<Claims, ApiError> {
    let token = extract_bearer(headers).ok_or_else(ApiError::unauthorized)?;
    let claims = verify_jwt(state, &token)?;
    check_block(state, &claims.sub, &claims.app)?;
    Ok(claims)
}

/// Block check for "name@app" then bare "app".
pub fn check_block(state: &AppState, name: &str, app: &str) -> Result<(), ApiError> {
    let cfg = state.config.read().unwrap();
    match cfg.blocked_status(name, app) {
        BlockStatus::None => Ok(()),
        _ => Err(ApiError::blocked()),
    }
}

// ---------------------------------------------------------------------------
// client IP (socket peer vs proxy headers)
// ---------------------------------------------------------------------------

/// The proxy-supplied client IP, when `network.trust_proxy_headers` is on:
/// X-Real-IP (set verbatim by our nginx), else the LAST entry of
/// X-Forwarded-For — the one appended by our own trusted proxy, which a
/// client cannot influence (earlier entries are client-controlled and only
/// informational). Values must parse as an IP; garbage is ignored.
fn proxy_ip(headers: &HeaderMap) -> Option<std::net::IpAddr> {
    if let Some(v) = headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        if let Ok(ip) = v.parse() {
            return Some(ip);
        }
    }
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())?
        .rsplit(',')
        .next()
        .map(str::trim)
        .and_then(|v| v.parse().ok())
}

/// Effective client IP for THROTTLING (no port): the proxy header IP when
/// trusted, else the socket peer IP.
pub fn effective_ip(state: &AppState, headers: &HeaderMap, addr: &std::net::SocketAddr) -> String {
    if state.trust_proxy_headers {
        if let Some(ip) = proxy_ip(headers) {
            return ip.to_string();
        }
    }
    addr.ip().to_string()
}

/// Effective client address for LOGGING: the proxy header IP (no port — the
/// proxy doesn't forward one), else the full socket peer `IP:PORT`.
pub fn effective_addr(
    state: &AppState,
    headers: &HeaderMap,
    addr: &std::net::SocketAddr,
) -> String {
    if state.trust_proxy_headers {
        if let Some(ip) = proxy_ip(headers) {
            return ip.to_string();
        }
    }
    addr.to_string()
}

/// Authenticate + permission check for a specific (action, db, coll).
/// `addr` is the peer socket address, logged on the debug trace line.
pub async fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    addr: &std::net::SocketAddr,
    action: &str,
    db: &str,
    coll: &str,
) -> Result<Claims, ApiError> {
    let claims = authenticate(state, headers)?;
    let allowed = state
        .perms
        .read()
        .unwrap()
        .allows(&claims.sub, &claims.app, action, db, coll);
    if !allowed {
        return Err(ApiError::new(
            ApiErrorKind::Forbidden,
            format!("no {action} permission on {db}.{coll}"),
        ));
    }
    // per-request trace line (only emitted when dashboard.log_level = debug);
    // identity stays last ("... as name@app") so the Logs tab facet parser
    // (state.rs log_identify) keeps working
    let from = effective_addr(state, headers, addr);
    tracing::debug!(
        "{action} /q/{db}/{coll} from {from} as {}@{}",
        claims.sub,
        claims.app
    );
    Ok(claims)
}

// ---------------------------------------------------------------------------
// per-request bookkeeping (no locks held across await)
// ---------------------------------------------------------------------------

/// Per-request bookkeeping (no locks held across await). `weight` is the
/// request's work units — 1 for ordinary requests, the document count for
/// batch inserts — so per-client rate accounting (rps/sparklines) reflects
/// write volume, not just request count.
fn record_request(state: &AppState, claims: &Claims, started: Instant, weight: u32) {
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    state.total_requests.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut ring) = state.latencies.lock() {
        ring.push_back(elapsed_ms);
        if ring.len() > 2048 {
            ring.pop_front();
        }
    }
    let now = crate::state::now_ms();
    let name_key = format!("name:{}@{}", claims.sub, claims.app);
    let app_key = format!("app:{}", claims.app);
    for key in [&name_key, &app_key] {
        let entry = state.clients.entry(key.clone()).or_insert_with(|| {
            if key.starts_with("app:") {
                ClientStats::new("", &claims.app)
            } else {
                ClientStats::new(&claims.sub, &claims.app)
            }
        });
        entry.total.fetch_add(weight as u64, Ordering::Relaxed);
        entry.last_seen.store(now, Ordering::Relaxed);
        if let Ok(mut lat) = entry.lat.lock() {
            lat.push_back(elapsed_ms);
            if lat.len() > 256 {
                lat.pop_front();
            }
        }
    }
}

/// The document limit this app gets right now (adaptive, clamped).
pub fn enforced_limit(state: &AppState, app: &str) -> u32 {
    state
        .limits
        .get(app)
        .map(|l| l.enforced)
        .unwrap_or_else(|| {
            state
                .config
                .read()
                .map(|c| c.rate_limit.max_limit)
                .unwrap_or(200)
        })
}

// ---------------------------------------------------------------------------
// GET /ls — list what the caller may GET
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ListParams {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    pub db: Option<String>,
}

/// Build + register a listing cursor (db names or collection names, sorted
/// by "name" asc).
fn make_list_cursor(state: &AppState, db_field: &str, coll_field: &str, last_name: &str) -> Cursor {
    let seq = state.cursor_seq.fetch_add(1, Ordering::Relaxed);
    let id = format!("c{:x}", seq);
    let now = crate::state::now_ms();
    state.cursors.insert(
        id.clone(),
        crate::state::CursorInfo {
            id: id.clone(),
            db: db_field.to_string(),
            coll: coll_field.to_string(),
            created_ms: now,
            last_used_ms: std::sync::atomic::AtomicI64::new(now),
            uses: std::sync::atomic::AtomicU64::new(0),
        },
    );
    Cursor {
        v: 1,
        id,
        db: db_field.to_string(),
        coll: coll_field.to_string(),
        sort: vec![("name".into(), 1i8)],
        last: vec![serde_json::to_string(last_name).unwrap_or_default()],
    }
}

fn cursor_last_name(cur: &Cursor) -> Result<Option<String>, ApiError> {
    let v: Value = serde_json::from_str(cur.last.first().map(String::as_str).unwrap_or(""))
        .map_err(|_| ApiError::new(ApiErrorKind::InvalidCursor, "malformed listing cursor"))?;
    v.as_str()
        .map(|s| s.to_string())
        .map(Some)
        .ok_or_else(|| ApiError::new(ApiErrorKind::InvalidCursor, "malformed listing cursor"))
}

pub async fn list_visible(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<crate::tls::MyAddr>,
    headers: HeaderMap,
    QueryBody(params): QueryBody<ListParams>,
) -> Result<Json<Value>, ApiError> {
    let started = Instant::now();
    let claims = authenticate(&state, &headers)?;
    let from = effective_addr(&state, &headers, &addr.0);
    tracing::debug!("GET /ls from {from} as {}@{}", claims.sub, claims.app);

    // ?db=X — the collections of one database
    if let Some(db) = &params.db {
        let all = dbq::list_databases(&state).await?;
        if !all.iter().any(|d| d == db) {
            return Err(ApiError::not_found(format!(
                "database '{db}' does not exist"
            )));
        }
        let all_colls = dbq::list_collections(&state, db).await?;
        let (visible_dbs, colls) = {
            let perms = state.perms.read().unwrap();
            let dbs = perms.listable_databases(&claims.sub, &claims.app, &[db.clone()]);
            let colls = perms.listable_collections(&claims.sub, &claims.app, db, &all_colls);
            (dbs, colls)
        };
        if visible_dbs.is_empty() {
            return Err(ApiError::new(
                ApiErrorKind::Forbidden,
                format!("no access to database '{db}'"),
            ));
        }
        record_request(&state, &claims, started, 1);
        return Ok(Json(json!({ "db": db, "collections": colls })));
    }

    // no ?db= — flat list of databases the caller may GET
    if let Some(0) = params.limit {
        return Err(ApiError::new(
            ApiErrorKind::InvalidLimit,
            "limit must be a positive integer",
        ));
    }
    let limit = requested_limit(&params.limit, &state, &claims.app);
    let mut db_names = dbq::list_databases(&state).await?;

    // cursor continuation (names are sorted ascending)
    if let Some(c) = &params.cursor {
        let cur = dbq::Cursor::decode(c)?;
        if cur.db != "*" || cur.coll != "*" {
            return Err(ApiError::new(
                ApiErrorKind::InvalidCursor,
                "wrong listing cursor",
            ));
        }
        dbq::touch_cursor(&state, &cur.id);
        if let Some(last) = cursor_last_name(&cur)? {
            db_names.retain(|d| d > &last);
        }
    }
    db_names.sort();

    let visible: Vec<String> = {
        let perms = state.perms.read().unwrap();
        perms.listable_databases(&claims.sub, &claims.app, &db_names)
    };

    let page = visible
        .iter()
        .take(limit as usize)
        .cloned()
        .collect::<Vec<_>>();
    let has_more = visible.len() as u32 > limit;

    let next_cursor = if has_more {
        Some(make_list_cursor(&state, "*", "*", page.last().unwrap()).encode())
    } else {
        None
    };

    record_request(&state, &claims, started, 1);
    Ok(Json(json!({
        "databases": page,
        "next_cursor": next_cursor,
        "has_more": has_more,
        "limit_applied": page.len(),
    })))
}

// ---------------------------------------------------------------------------
// GET /q/{db}/{coll}
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct FindParams {
    pub filter: Option<String>,
    pub limit: Option<u32>,
    pub sort: Option<String>,
    pub cursor: Option<String>,
    pub projection: Option<String>,
}

pub async fn find_docs(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<crate::tls::MyAddr>,
    headers: HeaderMap,
    Path((db, coll)): Path<(String, String)>,
    QueryBody(params): QueryBody<FindParams>,
) -> Result<Json<Value>, ApiError> {
    let started = Instant::now();
    check_path(&db, &coll)?;
    let claims = authorize(&state, &headers, &addr.0, "GET", &db, &coll).await?;
    let app = claims.app.clone();

    // --- parse user inputs ---
    let filter_doc = match &params.filter {
        Some(f) => parse_filter_json(f)?,
        None => Document::new(),
    };
    let sort_raw = match &params.sort {
        Some(s) => parse_sort_json(s)?,
        None => vec![],
    };
    if let Some(0) = params.limit {
        return Err(ApiError::new(
            ApiErrorKind::InvalidLimit,
            "limit must be a positive integer",
        ));
    }
    let requested = params.limit.unwrap_or(u32::MAX); // no limit -> adaptive limit applies

    // --- cursor handling ---
    let cursor = match &params.cursor {
        Some(c) => {
            let cur = dbq::Cursor::decode(c)?;
            if cur.db != db || cur.coll != coll {
                return Err(ApiError::new(
                    ApiErrorKind::InvalidCursor,
                    "cursor does not match this collection",
                ));
            }
            dbq::touch_cursor(&state, &cur.id);
            if !sort_raw.is_empty() && dbq::normalize_sort(&sort_raw) != cur.sort {
                return Err(ApiError::new(
                    ApiErrorKind::InvalidCursor,
                    "sort does not match cursor",
                ));
            }
            Some(cur)
        }
        None => None,
    };
    let sort = match &cursor {
        Some(c) => c.sort.clone(),
        None => dbq::normalize_sort(&sort_raw),
    };

    // --- projection ---
    let projection = match &params.projection {
        Some(s) => parse_projection_json(s)?,
        None => None,
    };
    let mongo_projection = dbq::projection_document(projection.as_ref(), &sort);
    let strip = dbq::projection_strip_fields(projection.as_ref(), &sort);

    // --- adaptive limit ---
    let enforced = enforced_limit(&state, &app);
    let effective = requested.min(enforced);
    let truncated = requested > enforced;

    let full_filter = dbq::build_filter(Some(filter_doc), cursor.as_ref())?;
    let (docs, has_more, last_doc) = dbq::find_page(
        &state,
        &db,
        &coll,
        full_filter,
        &sort,
        effective,
        mongo_projection,
        cursor.as_ref(),
    )
    .await?;

    // Arrays in the sort field cannot be represented by a keyset cursor:
    // Mongo sorts arrays element-wise (against scalars and other arrays), a
    // property no query operator can express — continuing past such a page
    // would silently drop or duplicate documents. Refuse loudly instead.
    if has_more {
        for d in docs.iter().chain(last_doc.iter()) {
            for (f, _) in &sort {
                if matches!(d.get(f), Some(bson::Bson::Array(_))) {
                    return Err(ApiError::new(
                        ApiErrorKind::BadRequest,
                        format!(
                            "cannot paginate: sort field {f:?} contains an array value (array sort order cannot be represented in a keyset cursor) — use a different sort or filter"
                        ),
                    ));
                }
            }
        }
    }

    let next_cursor = match (has_more, last_doc) {
        (true, Some(doc)) => Some(dbq::make_next_cursor(&state, &db, &coll, &sort, &doc)?.encode()),
        _ => None,
    };

    let out: Vec<Value> = docs
        .iter()
        .map(|d| dbq::bson_to_json_projected(d, &strip))
        .collect();
    record_request(&state, &claims, started, 1);
    Ok(Json(json!({
        "documents": out,
        "next_cursor": next_cursor,
        "has_more": has_more,
        "truncated": truncated,
        "limit_applied": effective,
        "count": out.len(),
    })))
}

// ---------------------------------------------------------------------------
// POST / PUT / PATCH / DELETE /q/{db}/{coll}
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct WriteBody {
    pub filter: Option<Value>,
    pub data: Option<Value>,
}

pub async fn insert_or_update(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<crate::tls::MyAddr>,
    headers: HeaderMap,
    Path((db, coll)): Path<(String, String)>,
    JsonBody(body): JsonBody<WriteBody>,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let started = Instant::now();
    check_path(&db, &coll)?;

    if body.filter.is_none() {
        // pure insert: JSON object -> insert_one, JSON array -> insert_many
        let claims = authorize(&state, &headers, &addr.0, "POST", &db, &coll).await?;
        let data = body
            .data
            .clone()
            .ok_or_else(|| ApiError::bad_request("missing data"))?;
        let b = dbq::json_to_bson(&data).map_err(ApiError::bad_request)?;
        return match b {
            bson::Bson::Document(doc) => {
                let id = dbq::insert_one(&state, &db, &coll, doc).await?;
                record_request(&state, &claims, started, 1);
                Ok((
                    axum::http::StatusCode::CREATED,
                    Json(json!({ "inserted_count": 1, "inserted_id": dbq::bson_to_json(&id) })),
                ))
            }
            bson::Bson::Array(items) => {
                let docs = batch_to_docs(items, state.max_insert_batch)?;
                let ids = dbq::insert_many(&state, &db, &coll, docs).await?;
                let n = ids.len();
                record_request(&state, &claims, started, n as u32);
                Ok((
                    axum::http::StatusCode::CREATED,
                    Json(json!({
                        "inserted_count": n,
                        "inserted_ids": ids.iter().map(dbq::bson_to_json).collect::<Vec<_>>(),
                    })),
                ))
            }
            _ => Err(ApiError::bad_request(
                "data must be a JSON object or an array of JSON objects",
            )),
        };
    }

    // update-many with $set
    let claims = authorize(&state, &headers, &addr.0, "POST", &db, &coll).await?;
    let (filter, data) = body_filter_data(&body)?;
    let r = dbq::update_many(&state, &db, &coll, filter, doc! { "$set": data }, false).await?;
    record_request(&state, &claims, started, 1);
    Ok((
        axum::http::StatusCode::OK,
        Json(json!({
            "matched_count": r.matched_count,
            "modified_count": r.modified_count,
        })),
    ))
}

/// Validate an insert batch (array `data`): non-empty, capped at
/// MAX_INSERT_BATCH, every element a JSON object, and no duplicate `_id`
/// within the batch — an ordered insert_many would otherwise half-apply the
/// batch on a duplicate (Mongo aborts at the first dup key, earlier docs
/// stay), so a clean 400 before Mongo avoids surprise partial writes.
fn batch_to_docs(items: Vec<bson::Bson>, max_batch: usize) -> Result<Vec<Document>, ApiError> {
    if items.is_empty() {
        return Err(ApiError::bad_request("insert batch must not be empty"));
    }
    if items.len() > max_batch {
        return Err(ApiError::bad_request(format!(
            "insert batch too large (max {max_batch} documents)"
        )));
    }
    let mut docs = Vec::with_capacity(items.len());
    let mut seen: HashSet<Value> = HashSet::with_capacity(items.len());
    for (i, item) in items.into_iter().enumerate() {
        let doc = dbq::require_object(&item, &format!("data[{i}]"))?.clone();
        if let Some(id) = doc.get("_id") {
            // canonical extended JSON == type-sensitive equality, matching
            // how MongoDB's _id index treats 1 vs 1.0 as different keys
            if !seen.insert(id.clone().into_canonical_extjson()) {
                return Err(ApiError::bad_request("duplicate _id within insert batch"));
            }
        }
        docs.push(doc);
    }
    Ok(docs)
}

pub async fn put_update(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<crate::tls::MyAddr>,
    headers: HeaderMap,
    Path((db, coll)): Path<(String, String)>,
    JsonBody(body): JsonBody<WriteBody>,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let started = Instant::now();
    check_path(&db, &coll)?;
    let claims = authorize(&state, &headers, &addr.0, "PUT", &db, &coll).await?;
    let (filter, data) = body_filter_data(&body)?;
    if filter.is_empty() {
        return Err(ApiError::bad_request(
            "PUT requires a filter (use POST without filter to insert)",
        ));
    }
    let r = dbq::update_many(&state, &db, &coll, filter, doc! { "$set": data }, false).await?;
    record_request(&state, &claims, started, 1);
    if r.matched_count == 0 {
        return Err(ApiError::not_found("no document matched the filter"));
    }
    Ok((
        axum::http::StatusCode::OK,
        Json(json!({
            "matched_count": r.matched_count,
            "modified_count": r.modified_count,
        })),
    ))
}

pub async fn patch_upsert(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<crate::tls::MyAddr>,
    headers: HeaderMap,
    Path((db, coll)): Path<(String, String)>,
    JsonBody(body): JsonBody<WriteBody>,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let started = Instant::now();
    check_path(&db, &coll)?;
    let claims = authorize(&state, &headers, &addr.0, "PATCH", &db, &coll).await?;

    // upsert-many: array `data`, no filter — per-doc upsert by _id (docs
    // without _id are plain inserts) in one bulkWrite round trip
    if let Some(Value::Array(items)) = &body.data {
        if body.filter.is_some() {
            return Err(ApiError::bad_request("batch upsert takes no filter"));
        }
        let docs = batch_to_docs(
            items
                .iter()
                .map(|v| dbq::json_to_bson(v).map_err(ApiError::bad_request))
                .collect::<Result<Vec<bson::Bson>, _>>()?,
            state.max_insert_batch,
        )?;
        let n = docs.len();
        let r = dbq::bulk_upsert(&state, &db, &coll, docs).await?;
        record_request(&state, &claims, started, n as u32);
        return Ok((
            axum::http::StatusCode::OK,
            Json(json!({
                "matched_count": r.matched_count,
                "modified_count": r.modified_count,
                "inserted_count": r.inserted_count,
                "upserted_count": r.upserted_count,
                "inserted_ids": r.inserted_ids.iter().map(dbq::bson_to_json).collect::<Vec<_>>(),
                "upserted_ids": r.upserted_ids.iter().map(dbq::bson_to_json).collect::<Vec<_>>(),
            })),
        ));
    }

    let (filter, data) = body_filter_data(&body)?;
    if filter.is_empty() {
        return Err(ApiError::bad_request("PATCH requires a filter"));
    }
    let r = dbq::update_many(&state, &db, &coll, filter, doc! { "$set": data }, true).await?;
    record_request(&state, &claims, started, 1);
    let upserted = r.upserted_id.is_some();
    let upserted_id = r.upserted_id.as_ref().map(dbq::bson_to_json);
    let status = if upserted {
        axum::http::StatusCode::CREATED
    } else {
        axum::http::StatusCode::OK
    };
    Ok((
        status,
        Json(json!({
            "matched_count": r.matched_count,
            "modified_count": r.modified_count,
            "upserted": upserted,
            "upserted_id": upserted_id,
        })),
    ))
}

pub async fn delete_docs(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<crate::tls::MyAddr>,
    headers: HeaderMap,
    Path((db, coll)): Path<(String, String)>,
    JsonBody(body): JsonBody<Value>,
) -> Result<Json<Value>, ApiError> {
    let started = Instant::now();
    check_path(&db, &coll)?;
    let claims = authorize(&state, &headers, &addr.0, "DELETE", &db, &coll).await?;
    let filter = match body.get("filter") {
        Some(f) => {
            if has_script_operator(f) {
                return Err(ApiError::new(
                    ApiErrorKind::InvalidFilter,
                    "server-side script operators ($where, $function) are not allowed",
                ));
            }
            match dbq::json_to_bson(f).map_err(|e| {
                ApiError::new(ApiErrorKind::InvalidFilter, format!("invalid filter: {e}"))
            })? {
                bson::Bson::Document(d) => d,
                _ => {
                    return Err(ApiError::new(
                        ApiErrorKind::InvalidFilter,
                        "filter must be a JSON object",
                    ));
                }
            }
        }
        None => return Err(ApiError::bad_request("missing filter")),
    };
    let r = dbq::delete_many(&state, &db, &coll, filter).await?;
    record_request(&state, &claims, started, 1);
    if r.deleted_count == 0 {
        return Err(ApiError::not_found("no document matched the filter"));
    }
    Ok(Json(json!({ "deleted_count": r.deleted_count })))
}

// ---------------------------------------------------------------------------
// GET/POST/DELETE /q/{db}/{coll}/indexes — list, ensure (idempotent), drop
//
// Permission model: GET = read the collection => see its indexes; ensure and
// drop both require the dedicated INDEX action — a schema-level capability,
// deliberately neither document-write (POST) nor document-delete (DELETE).
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct EnsureIndexBody {
    pub keys: Value,
    pub name: Option<String>,
    pub unique: Option<bool>,
    pub sparse: Option<bool>,
    pub expire_after_seconds: Option<u64>,
    pub partial_filter_expression: Option<Value>,
}

#[derive(Deserialize)]
pub struct DropIndexBody {
    pub name: String,
}

/// Index key document: non-empty object, each field 1 / -1 (JSON numbers
/// arrive as Int64) or an index-type string ("text", "2dsphere", "hashed",
/// ...). Everything else Mongo would reject server-side — fail with a clear
/// 400 instead of a mapped driver error.
fn parse_keys(v: &Value) -> Result<Document, ApiError> {
    let b = dbq::json_to_bson(v).map_err(ApiError::bad_request)?;
    let d = dbq::require_object(&b, "keys")?.clone();
    if d.is_empty() {
        return Err(ApiError::bad_request("keys must not be empty"));
    }
    for (f, dir) in &d {
        let ok = match dir {
            bson::Bson::Int32(i) => *i == 1 || *i == -1,
            bson::Bson::Int64(i) => *i == 1 || *i == -1,
            bson::Bson::String(_) => true,
            _ => false,
        };
        if !ok {
            return Err(ApiError::bad_request(format!(
                "invalid direction for key {f:?}: use 1, -1 or an index-type string"
            )));
        }
    }
    Ok(d)
}

/// One index as JSON: name + keys always; option fields only when set on
/// the server (keep the payload flat like the rest of /q).
fn index_json(i: &dbq::IndexInfo) -> Value {
    let mut o = json!({
        "name": i.name,
        "keys": dbq::bson_to_json(&bson::Bson::Document(i.keys.clone())),
    });
    if i.unique.unwrap_or(false) {
        o["unique"] = json!(true);
    }
    if i.sparse.unwrap_or(false) {
        o["sparse"] = json!(true);
    }
    if let Some(s) = i.expire_after_seconds {
        o["expire_after_seconds"] = json!(s);
    }
    if let Some(p) = &i.partial_filter_expression {
        o["partial_filter_expression"] = dbq::bson_to_json(&bson::Bson::Document(p.clone()));
    }
    o
}

pub async fn list_indexes(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<crate::tls::MyAddr>,
    headers: HeaderMap,
    Path((db, coll)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let started = Instant::now();
    check_path(&db, &coll)?;
    let claims = authorize(&state, &headers, &addr.0, "GET", &db, &coll).await?;
    let idx = dbq::list_indexes(&state, &db, &coll).await?;
    let out: Vec<Value> = idx.iter().map(index_json).collect();
    let n = out.len();
    record_request(&state, &claims, started, 1);
    Ok(Json(json!({ "indexes": out, "count": n })))
}

/// POST = ensure: 201 created / 200 already present / 409 same name or keys
/// taken by an incompatible index.
pub async fn ensure_index(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<crate::tls::MyAddr>,
    headers: HeaderMap,
    Path((db, coll)): Path<(String, String)>,
    JsonBody(body): JsonBody<EnsureIndexBody>,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let started = Instant::now();
    check_path(&db, &coll)?;
    let claims = authorize(&state, &headers, &addr.0, "INDEX", &db, &coll).await?;

    let keys = parse_keys(&body.keys)?;
    let partial_filter_expression = match &body.partial_filter_expression {
        Some(v) => {
            if has_script_operator(v) {
                return Err(ApiError::new(
                    ApiErrorKind::InvalidFilter,
                    "server-side script operators ($where, $function) are not allowed",
                ));
            }
            match dbq::json_to_bson(v).map_err(ApiError::bad_request)? {
                bson::Bson::Document(d) => Some(d),
                _ => {
                    return Err(ApiError::bad_request(
                        "partial_filter_expression must be a JSON object",
                    ));
                }
            }
        }
        None => None,
    };
    if let Some(name) = &body.name {
        if name.is_empty() {
            return Err(ApiError::bad_request("index name must not be empty"));
        }
    }

    let spec = dbq::IndexSpec {
        keys,
        name: body.name.clone(),
        unique: body.unique,
        sparse: body.sparse,
        expire_after_seconds: body.expire_after_seconds,
        partial_filter_expression,
    };
    let (status, payload) = match dbq::ensure_index(&state, &db, &coll, spec).await? {
        dbq::EnsureOutcome::Created(name) => (
            axum::http::StatusCode::CREATED,
            json!({ "created": true, "name": name }),
        ),
        dbq::EnsureOutcome::Exists(name) => (
            axum::http::StatusCode::OK,
            json!({ "created": false, "name": name }),
        ),
    };
    record_request(&state, &claims, started, 1);
    Ok((status, Json(payload)))
}

/// DELETE = drop by name (the name GET returns; unambiguous round-trip).
pub async fn drop_index(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<crate::tls::MyAddr>,
    headers: HeaderMap,
    Path((db, coll)): Path<(String, String)>,
    JsonBody(body): JsonBody<DropIndexBody>,
) -> Result<Json<Value>, ApiError> {
    let started = Instant::now();
    check_path(&db, &coll)?;
    let claims = authorize(&state, &headers, &addr.0, "INDEX", &db, &coll).await?;
    if body.name.is_empty() {
        return Err(ApiError::bad_request("index name must not be empty"));
    }
    dbq::drop_index_by_name(&state, &db, &coll, &body.name).await?;
    record_request(&state, &claims, started, 1);
    Ok(Json(json!({ "deleted": true, "name": body.name })))
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn check_path(db: &str, coll: &str) -> Result<(), ApiError> {
    if !valid_path_segment(db, true) || !valid_path_segment(coll, false) {
        return Err(ApiError::bad_request("invalid database or collection name"));
    }
    Ok(())
}

/// Recursively reject server-side script operators in client filters:
/// `$where` executes JavaScript on the server for every matched document and
/// `$function` runs arbitrary JS inside `$expr` — neither belongs in a REST
/// proxy (CPU DoS from any GET-permission client).
fn has_script_operator(v: &Value) -> bool {
    match v {
        Value::Object(m) => {
            m.contains_key("$where")
                || m.contains_key("$function")
                || m.values().any(has_script_operator)
        }
        Value::Array(a) => a.iter().any(has_script_operator),
        _ => false,
    }
}

/// convert body filter/data (both optional JSON objects) to BSON documents.
fn body_filter_data(body: &WriteBody) -> Result<(Document, Document), ApiError> {
    let filter = match &body.filter {
        Some(f) => {
            if has_script_operator(f) {
                return Err(ApiError::new(
                    ApiErrorKind::InvalidFilter,
                    "server-side script operators ($where, $function) are not allowed",
                ));
            }
            match dbq::json_to_bson(f).map_err(|e| {
                ApiError::new(ApiErrorKind::InvalidFilter, format!("invalid filter: {e}"))
            })? {
                bson::Bson::Document(d) => d,
                _ => {
                    return Err(ApiError::new(
                        ApiErrorKind::InvalidFilter,
                        "filter must be a JSON object",
                    ));
                }
            }
        }
        None => Document::new(),
    };
    let data = match &body.data {
        Some(d) => match dbq::json_to_bson(d).map_err(ApiError::bad_request)? {
            bson::Bson::Document(doc) => doc,
            _ => return Err(ApiError::bad_request("data must be a JSON object")),
        },
        None => return Err(ApiError::bad_request("missing data")),
    };
    Ok((filter, data))
}

fn parse_filter_json(s: &str) -> Result<Document, ApiError> {
    let v: Value = serde_json::from_str(s).map_err(|e| {
        ApiError::new(
            ApiErrorKind::InvalidFilter,
            format!("invalid filter JSON: {e}"),
        )
    })?;
    if has_script_operator(&v) {
        return Err(ApiError::new(
            ApiErrorKind::InvalidFilter,
            "server-side script operators ($where, $function) are not allowed",
        ));
    }
    match dbq::json_to_bson(&v)
        .map_err(|e| ApiError::new(ApiErrorKind::InvalidFilter, format!("invalid filter: {e}")))?
    {
        bson::Bson::Document(d) => Ok(d),
        _ => Err(ApiError::new(
            ApiErrorKind::InvalidFilter,
            "filter must be a JSON object",
        )),
    }
}

fn parse_sort_json(s: &str) -> Result<Vec<(String, i8)>, ApiError> {
    let v: Value = serde_json::from_str(s)
        .map_err(|e| ApiError::new(ApiErrorKind::InvalidSort, format!("invalid sort JSON: {e}")))?;
    dbq::parse_sort(&v)
}

fn parse_projection_json(s: &str) -> Result<Option<dbq::Projection>, ApiError> {
    let v: Value = serde_json::from_str(s).map_err(|e| {
        ApiError::new(
            ApiErrorKind::InvalidProjection,
            format!("invalid projection JSON: {e}"),
        )
    })?;
    dbq::parse_projection(&v)
}

fn requested_limit(limit: &Option<u32>, state: &AppState, app: &str) -> u32 {
    match limit {
        Some(0) | None => enforced_limit(state, app),
        Some(n) => (*n).min(enforced_limit(state, app)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    fn hdr(name: &'static str, val: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            axum::http::HeaderName::from_static(name),
            axum::http::HeaderValue::from_str(val).unwrap(),
        );
        h
    }

    #[test]
    fn proxy_ip_sources() {
        // X-Real-IP wins
        let mut h = hdr("x-real-ip", "1.2.3.4");
        h.insert(
            axum::http::HeaderName::from_static("x-forwarded-for"),
            axum::http::HeaderValue::from_str("9.9.9.9, 8.8.8.8").unwrap(),
        );
        assert_eq!(proxy_ip(&h).unwrap().to_string(), "1.2.3.4");
        // no X-Real-IP: LAST XFF entry (the one our proxy appended)
        assert_eq!(
            proxy_ip(&hdr("x-forwarded-for", "9.9.9.9, 8.8.8.8"))
                .unwrap()
                .to_string(),
            "8.8.8.8"
        );
        assert_eq!(
            proxy_ip(&hdr("x-forwarded-for", "::1"))
                .unwrap()
                .to_string(),
            "::1"
        );
        // garbage / empty / absent headers yield None
        assert_eq!(proxy_ip(&hdr("x-real-ip", "not-an-ip")), None);
        assert_eq!(proxy_ip(&hdr("x-forwarded-for", "garbage")), None);
        assert_eq!(proxy_ip(&HeaderMap::new()), None);
        // a garbage X-Real-IP falls through to XFF
        let mut h = hdr("x-real-ip", "not-an-ip");
        h.insert(
            axum::http::HeaderName::from_static("x-forwarded-for"),
            axum::http::HeaderValue::from_str("5.6.7.8").unwrap(),
        );
        assert_eq!(proxy_ip(&h).unwrap().to_string(), "5.6.7.8");
    }
}
