//! pernames — name-level permissions: two names, one app, different rights.
//!
//! Prerequisite: run setup_pernames first (creates the app with NO app-level
//! rights and two names: reader = GET only, writer = GET+POST). Both names
//! share the app's token; only the name_id in the identifier differs. Logs
//! in as reader@xdb-pernames and as writer@xdb-pernames, then shows the
//! layered evaluation (name.allow -> app.allow -> deny): the reader can read
//! but not write, the writer can do both.
//!
//! Usage:
//!   cargo run --manifest-path examples/Cargo.toml --bin pernames -- \
//!       [--token <app-token>] [--app xdb-pernames] [--base-url http://127.0.0.1:8000]

use serde_json::{json, Value};

const DB: &str = "xdb_pernames";
const COLL: &str = "items";

fn main() {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(60))
        .build();
    let (base, app, token) = args();

    // --- reader: GET allowed, POST denied ---
    let reader = login(&agent, &base, &format!("reader@{app}"), &token);
    println!("reader@{} authenticated", app);

    let (status, body) = get_q(&agent, &base, &reader, DB, COLL);
    println!(
        "  reader GET /q/{DB}/{COLL} -> {status} (expect 200), {} docs",
        body["count"]
    );
    let (status, body) = call(
        &agent,
        "POST",
        &format!("{base}/q/{DB}/{COLL}"),
        Some(&reader),
        Some(json!({ "data": { "text": "reader tries to write" } })),
    );
    println!("  reader POST /q/{DB}/{COLL} -> {status} (expect 403)");
    println!("    error: {} code: {}", body["error"], body["code"]);

    // --- writer: GET and POST allowed ---
    let writer = login(&agent, &base, &format!("writer@{app}"), &token);
    println!("\nwriter@{} authenticated", app);

    let (status, body) = call(
        &agent,
        "POST",
        &format!("{base}/q/{DB}/{COLL}"),
        Some(&writer),
        Some(json!({ "data": { "_id": "note-1", "text": "written by writer" } })),
    );
    println!("  writer POST /q/{DB}/{COLL} (insert) -> {status} (expect 201)");
    println!("    {}", body);
    let (status, body) = get_q(&agent, &base, &writer, DB, COLL);
    println!("  writer GET /q/{DB}/{COLL} -> {status} (expect 200)");
    println!(
        "    {}",
        serde_json::to_string_pretty(&body["documents"]).unwrap_or_default()
    );

    // --- and the reader can now READ what the writer wrote ---
    let (status, body) = get_q(&agent, &base, &reader, DB, COLL);
    println!(
        "\nreader GET again (writer's doc visible) -> {status}, {} docs",
        body["count"]
    );
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

fn get_q(agent: &ureq::Agent, base: &str, jwt: &str, db: &str, coll: &str) -> (u16, Value) {
    finish(
        agent
            .get(&format!("{base}/q/{db}/{coll}"))
            .set("Authorization", &format!("Bearer {jwt}"))
            .call(),
    )
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

fn args() -> (String, String, String) {
    let mut base = "http://127.0.0.1:8000".to_string();
    let mut app = "xdb-pernames".to_string();
    let mut token = std::env::var("XDB_TOKEN").unwrap_or_else(|_| "demo-token-change-me".into());
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--base-url" => base = it.next().expect("--base-url needs a value"),
            "--app" => app = it.next().expect("--app needs a value"),
            "--token" => token = it.next().expect("--token needs a value"),
            other => die(&format!("unknown option {other}")),
        }
    }
    (base, app, token)
}

fn die(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}
