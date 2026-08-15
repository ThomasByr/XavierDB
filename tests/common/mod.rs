//! Shared helpers for the XavierDB integration battery (tests/).
//!
//! Black-box HTTP tests against a RUNNING server + MongoDB. Requirements and
//! the run ritual are documented in `.agents/skills/build-run-test.md`; the
//! fixture world below is created by the bootstrap script.
//!
//! # Fixture world (pre-created, idempotent)
//!
//! Apps (in authorized_keys.yml, created via the dashboard perms API):
//! ```text
//! xdb_tb_main       token tb-main-secret-token      all verbs, all dbs, all colls
//! xdb_tb_restricted token tb-restricted-token       GET on * EXCEPT deny GET xdb_tb_secret
//! xdb_tb_ro         token tb-ro-secret-token        GET only xdb_tb_shared
//! xdb_tb_m1         token tb-m1-secret-token        GET on xdb_tb_* + DELETE on xdb_tb_shared,
//!                                                  deny GET xdb_tb_secret;
//!                                                  name m1user: deny DELETE xdb_tb_shared;
//!                                                  name m1user2: no name rules
//! xdb_tb_m2         token tb-m2-secret-token        POST+PATCH only, xdb_tb_shared (ingester)
//! xdb_tb_m3         token tb-m3-secret-token        GET only xdb_tb_shared / coll "public"
//! ```
//! Names: tester & tester2 @xdb_tb_main (full), ruser @xdb_tb_restricted,
//! reader & reader2 @xdb_tb_ro, m1user & m1user2 @xdb_tb_m1, u2 @xdb_tb_m2,
//! u3 @xdb_tb_m3.
//!
//! Databases (exist; each contains `seed` coll with `{_id:"seed-1", v:1}`):
//! xdb_tb_shared, xdb_tb_secret, xdb_tb_extra, xdb_tb_crud, xdb_tb_query,
//! xdb_tb_proj, xdb_tb_page, xdb_tb_edge.
//!
//! JWTs + the admin cookie are cached in `<temp>/xdb_tb_cache/*.{jwt,cookie}`
//! so a battery run performs ~0 Argon2id logins (every /auth costs ~5 s and
//! /auth + dashboard login share a 30/min per-IP throttle — never login per
//! test). `jwt()` probes the cache with a cheap authed call and re-logs-in
//! only when stale (401). JWT TTL is 90 min.
//!
//! # Rules for test files
//! - Never call /auth or /dashboard/api/login directly; use `jwt`/`dash_cookie`.
//! - State-mutating tests (perms/config/block) must take `suite_lock()` and
//!   restore state afterwards (unblock, re-add deleted apps, restore config).
//! - Each test uses its OWN collection names; seeding is idempotent (fixed
//!   `_id`, tolerate 409). Never rely on other tests having run.
//! - Assert on error `code` strings, not messages.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// constants / config
// ---------------------------------------------------------------------------

pub const APP_MAIN: &str = "xdb_tb_main";
pub const APP_RESTRICTED: &str = "xdb_tb_restricted";
pub const APP_RO: &str = "xdb_tb_ro";
pub const APP_M1: &str = "xdb_tb_m1";
pub const APP_M2: &str = "xdb_tb_m2";
pub const APP_M3: &str = "xdb_tb_m3";

pub const DB_SHARED: &str = "xdb_tb_shared";
pub const DB_SECRET: &str = "xdb_tb_secret";
pub const DB_EXTRA: &str = "xdb_tb_extra";

pub const TOKEN_MAIN: &str = "tb-main-secret-token";
pub const TOKEN_RESTRICTED: &str = "tb-restricted-token";
pub const TOKEN_RO: &str = "tb-ro-secret-token";
pub const TOKEN_M1: &str = "tb-m1-secret-token";
pub const TOKEN_M2: &str = "tb-m2-secret-token";
pub const TOKEN_M3: &str = "tb-m3-secret-token";

pub fn base() -> String {
    std::env::var("XDB_TB_BASE").unwrap_or_else(|_| "http://127.0.0.1:8000".to_string())
}

pub fn mongo_uri() -> String {
    std::env::var("XDB_TB_MONGO_URI").unwrap_or_else(|_| "mongodb://localhost:27017".to_string())
}

pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .build()
}

/// Panics (with a helpful message) if the server is not up after ~30 s.
pub fn ensure_server() {
    let mut last = String::new();
    for _ in 0..30 {
        match ureq::get(&format!("{}/health", base())).call() {
            Ok(r) if r.status() == 200 => return,
            Ok(r) => last = format!("status {}", r.status()),
            Err(e) => last = format!("{e}"),
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    panic!(
        "XavierDB server not reachable at {} ({last})\n\
         start MongoDB + the API first (see `.agents/skills/build-run-test.md`) and optionally set XDB_TB_BASE",
        base()
    );
}

/// Global mutex serializing state-mutating tests (perms/config/block).
pub fn suite_lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
}

pub fn err_code(body: &Value) -> String {
    body["code"].as_str().unwrap_or("").to_string()
}

pub fn err_status(body: &Value) -> u16 {
    body["status"].as_u64().unwrap_or(0) as u16
}

// ---------------------------------------------------------------------------
// identities & shared credentials
// ---------------------------------------------------------------------------

pub struct Identity {
    pub key: &'static str,
    pub identifier: &'static str,
    pub token: &'static str,
}

pub const IDENTITIES: &[Identity] = &[
    Identity {
        key: "main",
        identifier: "tester@xdb_tb_main",
        token: TOKEN_MAIN,
    },
    Identity {
        key: "main2",
        identifier: "tester2@xdb_tb_main",
        token: TOKEN_MAIN,
    },
    Identity {
        key: "ruser",
        identifier: "ruser@xdb_tb_restricted",
        token: TOKEN_RESTRICTED,
    },
    Identity {
        key: "reader",
        identifier: "reader@xdb_tb_ro",
        token: TOKEN_RO,
    },
    Identity {
        key: "reader2",
        identifier: "reader2@xdb_tb_ro",
        token: TOKEN_RO,
    },
    Identity {
        key: "m1user",
        identifier: "m1user@xdb_tb_m1",
        token: TOKEN_M1,
    },
    Identity {
        key: "m1user2",
        identifier: "m1user2@xdb_tb_m1",
        token: TOKEN_M1,
    },
    Identity {
        key: "u2",
        identifier: "u2@xdb_tb_m2",
        token: TOKEN_M2,
    },
    Identity {
        key: "u3",
        identifier: "u3@xdb_tb_m3",
        token: TOKEN_M3,
    },
];

fn cache_dir() -> PathBuf {
    std::env::temp_dir().join("xdb_tb_cache")
}

fn read_cache(name: &str) -> Option<String> {
    std::fs::read_to_string(cache_dir().join(name))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn write_cache(name: &str, v: &str) {
    let _ = std::fs::create_dir_all(cache_dir());
    let _ = std::fs::write(cache_dir().join(name), v);
}

/// Shared client JWT for an identity key ("main", "ruser", "u2", ...).
/// Probes the cache; re-logs-in only when stale. ~0 logins in a warm run.
pub fn jwt(key: &str) -> String {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let map = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(v) = map.lock().unwrap().get(key) {
        return v.clone();
    }
    let ident = IDENTITIES
        .iter()
        .find(|i| i.key == key)
        .unwrap_or_else(|| panic!("unknown identity key {key:?}"));
    let cached = read_cache(&format!("{key}.jwt"));
    let fresh = match &cached {
        Some(j) if jwt_valid(j) => j.clone(),
        _ => {
            let j = login(&ident.identifier, &ident.token);
            write_cache(&format!("{key}.jwt"), &j);
            j
        }
    };
    map.lock().unwrap().insert(key.to_string(), fresh.clone());
    fresh
}

fn jwt_valid(jwt: &str) -> bool {
    // 401 = invalid/expired; 403 = valid JWT but no permission (fine)
    let (status, _) = get(&agent(), &format!("{}/ls", base()), Some(jwt));
    status != 401
}

/// Fresh /auth call (no caching) — auth-flow tests only. Every call costs
/// ~5 s of Argon2id server-side; /auth + dashboard login share a 30/min
/// per-IP throttle. Use sparingly.
pub fn auth(agent: &ureq::Agent, identifier: &str, token: &str) -> (u16, Value) {
    post(
        agent,
        &format!("{}/auth", base()),
        None,
        None,
        Some(&json!({ "identifier": identifier, "token": token })),
    )
}

fn login(identifier: &str, token: &str) -> String {
    let (status, body) = auth(&agent(), identifier, token);
    assert_eq!(
        status, 200,
        "/auth failed for {identifier}: {status} {body} — is the fixture world bootstrapped? \
         (run the bootstrap script, see notebook `xavierdb-test-battery`)"
    );
    body["token"]
        .as_str()
        .expect("no token in /auth response")
        .to_string()
}

/// Shared dashboard session cookie (xdb_admin=...). Probes; re-logs-in when
/// stale (in-memory sessions die on server restart).
pub fn dash_cookie() -> String {
    static COOKIE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    let cell = COOKIE.get_or_init(|| Mutex::new(None));
    if let Some(c) = cell.lock().unwrap().as_ref() {
        return c.clone();
    }
    let cached = read_cache("admin.cookie");
    let fresh = match &cached {
        Some(c) if cookie_valid(c) => c.clone(),
        _ => {
            let c = dash_login();
            write_cache("admin.cookie", &c);
            c
        }
    };
    *cell.lock().unwrap() = Some(fresh.clone());
    fresh
}

fn cookie_valid(cookie: &str) -> bool {
    let (status, _) = dash_get(&agent(), cookie, "/dashboard/api/logs");
    status != 401
}

fn dash_login() -> String {
    let (user, pass) = dash_creds();
    let resp = ureq::post(&format!("{}/dashboard/api/login", base()))
        .send_json(json!({ "username": user, "password": pass }))
        .unwrap_or_else(|e| panic!("dashboard login request: {e}"));
    assert_eq!(resp.status(), 200, "dashboard login: {}", resp.status());
    resp.header("set-cookie")
        .map(|c| c.split(';').next().unwrap().trim().to_string())
        .unwrap_or_else(|| panic!("dashboard login response has no Set-Cookie"))
}

fn dash_creds() -> (String, String) {
    if let (Ok(u), Ok(p)) = (
        std::env::var("XDB_DASH_USER"),
        std::env::var("XDB_DASH_PASS"),
    ) {
        return (u, p);
    }
    // machine-local fallback: .pi/notes/credentials.md (gitignored)
    if let Ok(s) = std::fs::read_to_string(".pi/notes/credentials.md") {
        let mut user = String::new();
        let mut pass = String::new();
        for line in s.lines() {
            if let Some(v) = line
                .split("- password: `")
                .nth(1)
                .and_then(|v| v.split('`').next())
            {
                pass = v.to_string();
            }
            if let Some(v) = line
                .split("- username: `")
                .nth(1)
                .and_then(|v| v.split('`').next())
            {
                user = v.to_string();
            }
        }
        if !user.is_empty() && !pass.is_empty() {
            return (user, pass);
        }
    }
    panic!("dashboard credentials unavailable — set XDB_DASH_USER/XDB_DASH_PASS");
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

/// Generic request. bearer/cookie/body optional. Returns (status, JSON body).
pub fn call(
    agent: &ureq::Agent,
    method: &str,
    url: &str,
    bearer: Option<&str>,
    cookie: Option<&str>,
    body: Option<&Value>,
) -> (u16, Value) {
    let mut req = match method {
        "GET" => agent.get(url),
        "POST" => agent.post(url),
        "PUT" => agent.put(url),
        "PATCH" => agent.patch(url),
        "DELETE" => agent.delete(url),
        other => panic!("bad method {other}"),
    };
    if let Some(b) = bearer {
        req = req.set("Authorization", &format!("Bearer {b}"));
    }
    if let Some(c) = cookie {
        req = req.set("Cookie", c);
    }
    let res = match body {
        Some(b) => req.send_json(b),
        None => req.call(),
    };
    finish(res)
}

pub fn finish(res: Result<ureq::Response, ureq::Error>) -> (u16, Value) {
    match res {
        Ok(r) => {
            let status = r.status();
            let v = r.into_json().unwrap_or(Value::Null);
            (status, v)
        }
        Err(ureq::Error::Status(code, r)) => {
            let v = r.into_json().unwrap_or(Value::Null);
            (code, v)
        }
        Err(e) => panic!("request failed: {e}"),
    }
}

pub fn get(agent: &ureq::Agent, url: &str, bearer: Option<&str>) -> (u16, Value) {
    call(agent, "GET", url, bearer, None, None)
}

pub fn post(
    agent: &ureq::Agent,
    url: &str,
    bearer: Option<&str>,
    cookie: Option<&str>,
    body: Option<&Value>,
) -> (u16, Value) {
    call(agent, "POST", url, bearer, cookie, body)
}

// -- /q/ proxy -------------------------------------------------------------

pub fn get_q(
    agent: &ureq::Agent,
    jwt: &str,
    db: &str,
    coll: &str,
    params: &[(&str, &str)],
) -> (u16, Value) {
    let mut url = format!("{}/q/{db}/{coll}", base());
    if !params.is_empty() {
        let qs: Vec<String> = params
            .iter()
            .map(|(k, v)| format!("{k}={}", urlencode(v)))
            .collect();
        url.push('?');
        url.push_str(&qs.join("&"));
    }
    get(agent, &url, Some(jwt))
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn post_q(agent: &ureq::Agent, jwt: &str, db: &str, coll: &str, body: &Value) -> (u16, Value) {
    post(
        agent,
        &format!("{}/q/{db}/{coll}", base()),
        Some(jwt),
        None,
        Some(body),
    )
}

pub fn put_q(agent: &ureq::Agent, jwt: &str, db: &str, coll: &str, body: &Value) -> (u16, Value) {
    call(
        agent,
        "PUT",
        &format!("{}/q/{db}/{coll}", base()),
        Some(jwt),
        None,
        Some(body),
    )
}

pub fn patch_q(agent: &ureq::Agent, jwt: &str, db: &str, coll: &str, body: &Value) -> (u16, Value) {
    call(
        agent,
        "PATCH",
        &format!("{}/q/{db}/{coll}", base()),
        Some(jwt),
        None,
        Some(body),
    )
}

pub fn delete_q(
    agent: &ureq::Agent,
    jwt: &str,
    db: &str,
    coll: &str,
    body: &Value,
) -> (u16, Value) {
    call(
        agent,
        "DELETE",
        &format!("{}/q/{db}/{coll}", base()),
        Some(jwt),
        None,
        Some(body),
    )
}

pub fn ls(agent: &ureq::Agent, jwt: &str, params: &[(&str, &str)]) -> (u16, Value) {
    let mut url = format!("{}/ls", base());
    if !params.is_empty() {
        let qs: Vec<String> = params
            .iter()
            .map(|(k, v)| format!("{k}={}", urlencode(v)))
            .collect();
        url.push('?');
        url.push_str(&qs.join("&"));
    }
    get(agent, &url, Some(jwt))
}

pub fn health(agent: &ureq::Agent) -> (u16, Value) {
    get(agent, &format!("{}/health", base()), None)
}

/// The server's effective insert-batch cap, as published top-level in the
/// public /health document (MAX_INSERT_BATCH env, static per process).
pub fn max_insert_batch(agent: &ureq::Agent) -> usize {
    let (s, b) = health(agent);
    assert_eq!(s, 200, "{b}");
    b["max_insert_batch"]
        .as_u64()
        .expect("max_insert_batch in /health") as usize
}

// -- dashboard -------------------------------------------------------------

pub fn dash_get(agent: &ureq::Agent, cookie: &str, path: &str) -> (u16, Value) {
    call(
        agent,
        "GET",
        &format!("{}{path}", base()),
        None,
        Some(cookie),
        None,
    )
}

pub fn dash_post(
    agent: &ureq::Agent,
    cookie: &str,
    path: &str,
    body: Option<&Value>,
) -> (u16, Value) {
    call(
        agent,
        "POST",
        &format!("{}{path}", base()),
        None,
        Some(cookie),
        body,
    )
}

// ---------------------------------------------------------------------------
// seeding
// ---------------------------------------------------------------------------

/// Idempotent insert: fixed `_id`; 201 (inserted) and 409 (already there)
/// both count as success. Panics on anything else.
pub fn seed(agent: &ureq::Agent, jwt: &str, db: &str, coll: &str, id: &str, doc: Value) {
    let mut d = doc.clone();
    if d.get("_id").is_none() {
        d["_id"] = json!(id);
    }
    let (status, body) = post_q(agent, jwt, db, coll, &json!({ "data": d }));
    assert!(
        status == 201 || status == 409,
        "seed {db}/{coll} {id}: {status} {body}"
    );
}

/// Delete every document in a collection (ignores 404 = already empty).
pub fn clear_coll(agent: &ureq::Agent, jwt: &str, db: &str, coll: &str) {
    let (status, _) = delete_q(agent, jwt, db, coll, &json!({ "filter": {} }));
    assert!(
        status == 200 || status == 404,
        "clear {db}/{coll}: {status}"
    );
}

/// Number of documents in a collection via full-scroll GET (limit 200 = the
/// max enforced limit; multi-page for > 200 docs — not needed by tests).
pub fn count_docs(agent: &ureq::Agent, jwt: &str, db: &str, coll: &str) -> usize {
    let (status, body) = get_q(agent, jwt, db, coll, &[]);
    assert_eq!(status, 200, "count {db}/{coll}: {body}");
    body["documents"].as_array().map(|a| a.len()).unwrap_or(0)
}
