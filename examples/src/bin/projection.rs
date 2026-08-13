//! projection — the `projection` parameter of GET /q/{db}/{coll}.
//!
//! Prerequisite: run setup_projection first (creates the app + rights).
//! Logs in as demo@xdb-projection, seeds a few documents and demonstrates
//! include projections, exclude projections, `_id: 0`, and the 400
//! INVALID_PROJECTION error for a mixed {1,0} spec. Every response is
//! printed so the shape is visible.
//!
//! Usage:
//!   cargo run --manifest-path examples/Cargo.toml --bin projection -- \
//!       [--token <app-token>] [--app xdb-projection] [--name demo] [--base-url http://127.0.0.1:8000]

use serde_json::{json, Value};

const DB: &str = "xdb_projection";
const COLL: &str = "people";

fn main() {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(60))
        .build();
    let (base, app, name, token) = args();
    let jwt = login(&agent, &base, &format!("{name}@{app}"), &token);
    println!("authenticated as {name}@{app}\n");

    seed(&agent, &base, &jwt);

    // include projection: only name/age, no _id
    let (status, body) = get_q(
        &agent,
        &base,
        &jwt,
        DB,
        COLL,
        &[
            ("projection", r#"{"name":1,"age":1,"_id":0}"#),
            ("sort", r#"{"age":1}"#),
        ],
    );
    println!("projection {{name:1, age:1, _id:0}} (sorted by age) -> {status}");
    pprint(&body["documents"]);

    // exclude projection: everything except city and _id
    let (status, body) = get_q(
        &agent,
        &base,
        &jwt,
        DB,
        COLL,
        &[("projection", r#"{"city":0,"_id":0}"#)],
    );
    println!("\nprojection {{city:0, _id:0}} -> {status}");
    pprint(&body["documents"]);

    // include projection without _id: only the requested field remains
    let (status, body) = get_q(
        &agent,
        &base,
        &jwt,
        DB,
        COLL,
        &[("projection", r#"{"name":1}"#)],
    );
    println!("\nprojection {{name:1}} (no _id:0, no sort) -> {status}");
    pprint(&body["documents"]);

    // mixing inclusion and exclusion is rejected
    let (status, body) = get_q(
        &agent,
        &base,
        &jwt,
        DB,
        COLL,
        &[("projection", r#"{"name":1,"age":0}"#)],
    );
    println!("\nprojection {{name:1, age:0}} (mixed) -> {status}");
    println!("  error: {}", body["error"]);
    println!("  code:  {}", body["code"]);
}

/// Seed a few documents with fixed string _ids, so re-runs are no-ops
/// (the duplicate-key 409 is expected and ignored).
fn seed(agent: &ureq::Agent, base: &str, jwt: &str) {
    let docs = [
        json!({"_id":"p1", "name":"Ada",    "age":36, "city":"London",    "role":"analyst"}),
        json!({"_id":"p2", "name":"Grace",  "age":44, "city":"New York"}),
        json!({"_id":"p3", "name":"Edsger", "age":31, "city":"Amsterdam"}),
        json!({"_id":"p4", "name":"Linus",  "age":28, "city":"Helsinki"}),
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
    let mut app = "xdb-projection".to_string();
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
