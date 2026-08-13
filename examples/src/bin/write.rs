//! write — the write verbs of /q/{db}/{coll}: insert, update, PUT, PATCH, DELETE.
//!
//! Prerequisite: run setup_write first (creates the app + rights). Logs in as
//! demo@xdb-write and exercises every write verb on xdb_write/items:
//!   POST without filter  -> insert (201, returns the generated _id)
//!   POST with filter     -> update-many, data is auto-wrapped in $set
//!   PUT                  -> update-many, 404 when nothing matched
//!   PATCH                -> upsert (201 inserted / 200 updated)
//!   DELETE               -> delete-many, 404 when nothing matched
//!
//! Usage:
//!   cargo run --manifest-path examples/Cargo.toml --bin write -- \
//!       [--token <app-token>] [--app xdb-write] [--name demo] [--base-url http://127.0.0.1:8000]

use serde_json::{json, Value};

const DB: &str = "xdb_write";
const COLL: &str = "items";

fn main() {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(60))
        .build();
    let (base, app, name, token) = args();
    let jwt = login(&agent, &base, &format!("{name}@{app}"), &token);
    println!("authenticated as {name}@{app}\n");

    // 1. POST without filter = insert; the server generates an _id
    let (status, body) = call(
        &agent,
        "POST",
        &format!("{base}/q/{DB}/{COLL}"),
        Some(&jwt),
        Some(json!({ "data": { "sku": "A-1", "qty": 5 } })),
    );
    println!("1. insert {{sku:A-1, qty:5}} -> {status}");
    println!("   {}", body);

    // 2. POST with filter = update-many; `data` is auto-wrapped in $set
    let (status, body) = call(
        &agent,
        "POST",
        &format!("{base}/q/{DB}/{COLL}"),
        Some(&jwt),
        Some(json!({ "filter": { "sku": "A-1" }, "data": { "qty": 6 } })),
    );
    println!("2. update {{filter: sku:A-1, data: qty:6}} -> {status}");
    println!("   {}", body);

    // 3. PUT = update-many, 404 when nothing matched
    let (status, body) = call(
        &agent,
        "PUT",
        &format!("{base}/q/{DB}/{COLL}"),
        Some(&jwt),
        Some(json!({ "filter": { "sku": "A-1" }, "data": { "qty": 7 } })),
    );
    println!("3. PUT {{filter: sku:A-1, data: qty:7}} -> {status}");
    println!("   {}", body);
    let (status, body) = call(
        &agent,
        "PUT",
        &format!("{base}/q/{DB}/{COLL}"),
        Some(&jwt),
        Some(json!({ "filter": { "sku": "no-such-sku" }, "data": { "qty": 1 } })),
    );
    println!("   PUT (nothing matched) -> {status}");
    println!("   error: {} code: {}", body["error"], body["code"]);

    // 4. PATCH = upsert: 201 when it inserts, 200 when it updates
    let (status, body) = call(
        &agent,
        "PATCH",
        &format!("{base}/q/{DB}/{COLL}"),
        Some(&jwt),
        Some(json!({ "filter": { "sku": "B-2" }, "data": { "qty": 3 } })),
    );
    println!("4. PATCH upsert {{filter: sku:B-2, data: qty:3}} -> {status}");
    println!("   {}", body);
    let (status, body) = call(
        &agent,
        "PATCH",
        &format!("{base}/q/{DB}/{COLL}"),
        Some(&jwt),
        Some(json!({ "filter": { "sku": "B-2" }, "data": { "qty": 4 } })),
    );
    println!("   PATCH again (now matches) -> {status}");
    println!("   {}", body);

    // 5. DELETE, then DELETE again -> 404
    let (status, body) = call(
        &agent,
        "DELETE",
        &format!("{base}/q/{DB}/{COLL}"),
        Some(&jwt),
        Some(json!({ "filter": { "sku": "B-2" } })),
    );
    println!("5. DELETE {{filter: sku:B-2}} -> {status}");
    println!("   {}", body);
    let (status, body) = call(
        &agent,
        "DELETE",
        &format!("{base}/q/{DB}/{COLL}"),
        Some(&jwt),
        Some(json!({ "filter": { "sku": "B-2" } })),
    );
    println!("   DELETE again -> {status}");
    println!("   error: {} code: {}", body["error"], body["code"]);

    // 6. final state of the collection
    let (status, body) = get_q(&agent, &base, &jwt, DB, COLL, &[]);
    println!("\n6. final GET -> {status}: {} document(s)", body["count"]);
    pprint(&body["documents"]);
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

fn get_q(
    agent: &ureq::Agent,
    base: &str,
    jwt: &str,
    db: &str,
    coll: &str,
    params: &[(&str, &str)],
) -> (u16, Value) {
    let mut req = agent
        .get(&format!("{base}/q/{db}/{coll}"))
        .set("Authorization", &format!("Bearer {jwt}"));
    for (k, v) in params {
        req = req.query(k, v);
    }
    finish(req.call())
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
    let mut app = "xdb-write".to_string();
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
