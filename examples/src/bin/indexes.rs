//! indexes — GET/POST/DELETE /q/{db}/{coll}/indexes.
//!
//! Prerequisite: run setup_indexes first (creates the app + rights).
//! Logs in as demo@xdb-indexes, seeds a few documents (creating the
//! collection) and walks the whole index lifecycle: list (404 before the
//! collection exists, then _id_ only), ensure (201 created, 200 idempotent
//! re-ensure), unique enforcement on insert (409), both 409 conflict shapes
//! (same keys different options / same name different keys), a TTL index,
//! the final listing with option fields, drop (200), drop again (404) and
//! the refused `_id_` drop (400). Every response is printed so the shape is
//! visible. Safe to re-run: seeding ignores duplicate-key 409s and every
//! ensure/drop is idempotent or re-created from scratch.
//!
//! Usage:
//!   cargo run --manifest-path examples/Cargo.toml --bin indexes -- \
//!       [--token <app-token>] [--app xdb-indexes] [--name demo] [--base-url http://127.0.0.1:8000]

use serde_json::{json, Value};

const DB: &str = "xdb_indexes";
const COLL: &str = "orders";

fn main() {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(60))
        .build();
    let (base, app, name, token) = args();
    let jwt = login(&agent, &base, &format!("{name}@{app}"), &token);
    println!("authenticated as {name}@{app}\n");

    let idx_url = format!("{base}/q/{DB}/{COLL}/indexes");

    // before any insert the collection does not exist -> listing is a 404
    // (on re-runs the collection exists; either outcome is fine here)
    let (status, body) = call(&agent, "GET", &idx_url, Some(&jwt), None);
    println!("GET indexes (fresh collection) -> {status}");
    if status == 404 {
        println!("  error: {}", body["error"]);
        println!("  code:  {}", body["code"]);
    } else {
        pprint(&body["indexes"]);
    }
    println!();

    seed(&agent, &base, &jwt);

    // a collection starts with just the mandatory _id_ index
    // (re-runs still list the indexes left over from previous runs)
    let (_, body) = call(&agent, "GET", &idx_url, Some(&jwt), None);
    println!("GET indexes after seeding ->");
    pprint(&body["indexes"]);

    // ensure a compound index: 201 created (200 on re-runs)
    let (status, body) = call(
        &agent,
        "POST",
        &idx_url,
        Some(&jwt),
        Some(json!({ "keys": { "customer": 1, "created": -1 }, "name": "customer_created" })),
    );
    println!("\nPOST ensure customer_created -> {status}");
    pprint(&body);

    // same keys, same options -> idempotent 200 created:false
    let (status, body) = call(
        &agent,
        "POST",
        &idx_url,
        Some(&jwt),
        Some(json!({ "keys": { "customer": 1, "created": -1 }, "name": "customer_created" })),
    );
    println!("\nPOST ensure customer_created again -> {status}");
    pprint(&body);

    // a unique index on email
    let (status, body) = call(
        &agent,
        "POST",
        &idx_url,
        Some(&jwt),
        Some(json!({ "keys": { "email": 1 }, "name": "uniq_email", "unique": true })),
    );
    println!("\nPOST ensure uniq_email {{email:1}} unique -> {status}");
    pprint(&body);

    // the unique index bites: a second document with a taken email -> 409
    let (status, body) = call(
        &agent,
        "POST",
        &format!("{base}/q/{DB}/{COLL}"),
        Some(&jwt),
        Some(
            json!({ "data": { "_id": "o3", "customer": "c2", "created": "2026-08-20", "email": "c1@x.io" } }),
        ),
    );
    println!("\ninsert duplicate email (blocked by uniq_email) -> {status}");
    println!("  error: {}", body["error"]);
    println!("  code:  {}", body["code"]);

    // same keys, different options -> 409
    let (status, body) = call(
        &agent,
        "POST",
        &idx_url,
        Some(&jwt),
        Some(json!({ "keys": { "customer": 1, "created": -1 }, "unique": true })),
    );
    println!("\nPOST same keys, different options (unique) -> {status}");
    println!("  error: {}", body["error"]);
    println!("  code:  {}", body["code"]);

    // same name, different keys -> 409
    let (status, body) = call(
        &agent,
        "POST",
        &idx_url,
        Some(&jwt),
        Some(json!({ "keys": { "region": 1 }, "name": "customer_created" })),
    );
    println!("\nPOST same name, different keys -> {status}");
    println!("  error: {}", body["error"]);
    println!("  code:  {}", body["code"]);

    // a TTL index: expire_after_seconds shows up in the listing
    let (status, body) = call(
        &agent,
        "POST",
        &idx_url,
        Some(&jwt),
        Some(
            json!({ "keys": { "expires_at": 1 }, "name": "ttl_expires", "expire_after_seconds": 86400 }),
        ),
    );
    println!("\nPOST ensure ttl_expires {{expires_at:1}} TTL 86400s -> {status}");
    pprint(&body);

    // full listing: option fields (unique / expire_after_seconds) only when set
    let (_, body) = call(&agent, "GET", &idx_url, Some(&jwt), None);
    println!("\nGET indexes (final listing) -> count: {}", body["count"]);
    pprint(&body["indexes"]);

    // drop by the name GET returns -> 200
    let (status, body) = call(
        &agent,
        "DELETE",
        &idx_url,
        Some(&jwt),
        Some(json!({ "name": "uniq_email" })),
    );
    println!("\nDELETE uniq_email -> {status}");
    pprint(&body);

    // dropping it again -> 404
    let (status, body) = call(
        &agent,
        "DELETE",
        &idx_url,
        Some(&jwt),
        Some(json!({ "name": "uniq_email" })),
    );
    println!("\nDELETE uniq_email again -> {status}");
    println!("  error: {}", body["error"]);
    println!("  code:  {}", body["code"]);

    // the mandatory _id_ index can never be dropped -> 400
    let (status, body) = call(
        &agent,
        "DELETE",
        &idx_url,
        Some(&jwt),
        Some(json!({ "name": "_id_" })),
    );
    println!("\nDELETE _id_ (refused) -> {status}");
    println!("  error: {}", body["error"]);
    println!("  code:  {}", body["code"]);
}

/// Seed a few documents with fixed string _ids, so re-runs are no-ops
/// (the duplicate-key 409 is expected and ignored). No `expires_at` dates
/// are seeded: the TTL index on it never fires, it exists only to be listed.
fn seed(agent: &ureq::Agent, base: &str, jwt: &str) {
    let docs = [
        json!({"_id":"o1", "customer":"c1", "created":"2026-08-18", "email":"c1@x.io"}),
        json!({"_id":"o2", "customer":"c2", "created":"2026-08-19", "email":"c2@x.io"}),
    ];
    for d in &docs {
        let (status, _) = call(
            agent,
            "POST",
            &format!("{base}/q/{DB}/{COLL}"),
            Some(jwt),
            Some(json!({ "data": d })),
        );
        match status {
            201 => {}
            409 => {} // already seeded by a previous run
            other => die(&format!("seed insert failed: {other}")),
        }
    }
    println!("seeded {} documents\n", docs.len());
}

/// POST /auth -> the JWT to send as a Bearer token.
fn login(agent: &ureq::Agent, base: &str, identifier: &str, token: &str) -> String {
    let resp = agent
        .post(&format!("{base}/auth"))
        .send_json(json!({ "identifier": identifier, "token": token }))
        .unwrap_or_else(|e| die(&format!("POST /auth: {e}")));
    let body: Value = resp.into_json().unwrap_or_default();
    body["token"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| die(&format!("no token in /auth response: {body}")))
}

fn call(
    agent: &ureq::Agent,
    method: &str,
    url: &str,
    bearer: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    let mut req = match method {
        "GET" => agent.get(url),
        "POST" => agent.post(url),
        "PUT" => agent.put(url),
        "PATCH" => agent.patch(url),
        "DELETE" => agent.delete(url),
        other => die(&format!("bad method {other}")),
    };
    if let Some(b) = bearer {
        req = req.set("Authorization", &format!("Bearer {b}"));
    }
    let res = match body {
        Some(b) => req.send_json(b),
        None => req.call(),
    };
    finish(res)
}

fn finish(res: Result<ureq::Response, ureq::Error>) -> (u16, Value) {
    match res {
        Ok(resp) => {
            let status = resp.status();
            let v = resp.into_json().unwrap_or(Value::Null);
            (status, v)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let v = resp.into_json().unwrap_or(Value::Null);
            (code, v)
        }
        Err(e) => die(&format!("request failed: {e}")),
    }
}

fn pprint(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

fn args() -> (String, String, String, String) {
    let mut base = "http://127.0.0.1:8000".to_string();
    let mut app = "xdb-indexes".to_string();
    let mut name = "demo".to_string();
    let mut token = std::env::var("XDB_TOKEN").unwrap_or_else(|_| "demo-token-change-me".into());
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--base-url" => base = it.next().expect("--base-url needs a value"),
            "--app" => app = it.next().expect("--app needs a value"),
            "--name" => name = it.next().expect("--name needs a value"),
            "--token" => token = it.next().expect("--token needs a value"),
            other => die(&format!("unknown option {other}")),
        }
    }
    (base, app, name, token)
}

fn die(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}
