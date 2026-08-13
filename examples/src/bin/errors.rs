//! errors — the error contract: every failure is {error, code, status}.
//!
//! Prerequisite: run setup_errors first (creates the app with GET on db1 and
//! writes on xdb_errors only). Logs in as demo@xdb-errors — first with a
//! wrong token, then with the right one — and triggers each error class:
//!   401 UNAUTHORIZED  wrong token on /auth
//!   403 FORBIDDEN     action not granted (POST on db1, a GET-only db)
//!   404 NOT_FOUND     PUT/DELETE matched nothing
//!   409 CONFLICT      duplicate _id on insert
//! Each step prints the raw error body so the shape is visible.
//!
//! Usage:
//!   cargo run --manifest-path examples/Cargo.toml --bin errors -- \
//!       [--token <app-token>] [--app xdb-errors] [--name demo] [--base-url http://127.0.0.1:8000]

use serde_json::{json, Value};

const DB: &str = "xdb_errors";
const COLL: &str = "items";

fn main() {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(60))
        .build();
    let (base, app, name, token) = args();
    let identifier = format!("{name}@{app}");

    // 401: wrong token (verified against the real Argon2id hash, ~5s)
    let resp = agent
        .post(&format!("{base}/auth"))
        .send_json(json!({ "identifier": identifier, "token": "wrong-token" }));
    let (status, body) = finish(resp);
    println!("1. /auth with wrong token -> {status} (expect 401)");
    println!("   {body}");

    // good login
    let jwt = login(&agent, &base, &identifier, &token);
    println!("\n2. /auth with correct token -> 200, JWT received");

    // 403: the app has GET on db1 only — POST must be refused
    let (status, body) = call(
        &agent,
        "POST",
        &format!("{base}/q/db1/items"),
        Some(&jwt),
        Some(json!({ "data": { "n": 1 } })),
    );
    println!("\n3. POST /q/db1/items (GET-only db) -> {status} (expect 403)");
    println!("   {}", body);

    // 404: PUT matched nothing
    let (status, body) = call(
        &agent,
        "PUT",
        &format!("{base}/q/{DB}/{COLL}"),
        Some(&jwt),
        Some(json!({ "filter": { "sku": "no-such-sku" }, "data": { "v": 1 } })),
    );
    println!("\n4. PUT /q/{DB}/{COLL} (nothing matched) -> {status} (expect 404)");
    println!("   {}", body);

    // 409: duplicate _id — insert the same document twice
    let (status, body) = call(
        &agent,
        "POST",
        &format!("{base}/q/{DB}/{COLL}"),
        Some(&jwt),
        Some(json!({ "data": { "_id": "dup-demo", "v": 1 } })),
    );
    println!("\n5. insert {{_id: dup-demo}} -> {status} (expect 201)");
    println!("   {}", body);
    let (status, body) = call(
        &agent,
        "POST",
        &format!("{base}/q/{DB}/{COLL}"),
        Some(&jwt),
        Some(json!({ "data": { "_id": "dup-demo", "v": 1 } })),
    );
    println!("   insert {{_id: dup-demo}} again -> {status} (expect 409)");
    println!("   {}", body);

    // 404: DELETE matched nothing
    let (status, body) = call(
        &agent,
        "DELETE",
        &format!("{base}/q/{DB}/{COLL}"),
        Some(&jwt),
        Some(json!({ "filter": { "sku": "no-such-sku" } })),
    );
    println!("\n6. DELETE /q/{DB}/{COLL} (nothing matched) -> {status} (expect 404)");
    println!("   {}", body);
}

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

fn args() -> (String, String, String, String) {
    let mut base = "http://127.0.0.1:8000".to_string();
    let mut app = "xdb-errors".to_string();
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
