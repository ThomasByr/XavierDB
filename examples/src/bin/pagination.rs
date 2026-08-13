//! pagination — keyset cursor pagination on GET /q/{db}/{coll}.
//!
//! Prerequisite: run setup_pagination first (creates the app + rights).
//! Logs in as demo@xdb-pagination, seeds 10 documents and walks the whole
//! collection with limit=3, following `next_cursor` until `has_more` is
//! false. The cursor is opaque — the server decides the keyset; the client
//! just echoes it back (passing the same sort keeps it valid).
//!
//! Usage:
//!   cargo run --manifest-path examples/Cargo.toml --bin pagination -- \
//!       [--token <app-token>] [--app xdb-pagination] [--name demo] [--base-url http://127.0.0.1:8000]

use serde_json::{json, Value};

const DB: &str = "xdb_pagination";
const COLL: &str = "items";
const SORT: &str = r#"{"n":1}"#;
const LIMIT: u32 = 3;

fn main() {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(60))
        .build();
    let (base, app, name, token) = args();
    let jwt = login(&agent, &base, &format!("{name}@{app}"), &token);
    println!("authenticated as {name}@{app}\n");

    seed(&agent, &base, &jwt);

    // page 1: the sort must be sent with the request; the cursor then
    // carries the keyset for every continuation page
    let mut page = 1u32;
    let mut total = 0u32;
    let mut cursor: Option<String> = None;
    loop {
        let limit = LIMIT.to_string();
        let params: Vec<(&str, &str)> = match &cursor {
            Some(c) => vec![("sort", SORT), ("cursor", c), ("limit", &limit)],
            None => vec![("sort", SORT), ("limit", &limit)],
        };
        let (status, body) = get_q(&agent, &base, &jwt, DB, COLL, &params);
        if status != 200 {
            die(&format!("GET /q/{DB}/{COLL} failed: {status} {body}"));
        }
        let docs = body["documents"].as_array().cloned().unwrap_or_default();
        let has_more = body["has_more"].as_bool().unwrap_or(false);
        let n: Vec<String> = docs
            .iter()
            .map(|d| format!("{}={}", d["n"], d["name"]))
            .collect();
        println!(
            "page {page}: {} docs [{}] has_more={has_more}",
            docs.len(),
            n.join(", ")
        );
        total += docs.len() as u32;
        if !has_more {
            break;
        }
        cursor = body["next_cursor"].as_str().map(str::to_string);
        page += 1;
    }
    println!("\nwalked {page} pages, {total} documents total");
}

/// Seed 10 documents with fixed string _ids, so re-runs are no-ops
/// (the duplicate-key 409 is expected and ignored).
fn seed(agent: &ureq::Agent, base: &str, jwt: &str) {
    let mut docs = Vec::new();
    for n in 1..=10u32 {
        docs.push(json!({ "_id": format!("i{n:02}"), "n": n, "name": format!("item {n}") }));
    }
    for d in &docs {
        let (status, _) = call(
            agent,
            "POST",
            &format!("{base}/q/{DB}/{COLL}"),
            Some(jwt),
            Some(json!({ "data": d })),
        );
        match status {
            201 | 409 => {} // 409 = already seeded by a previous run
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
    let mut app = "xdb-pagination".to_string();
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
