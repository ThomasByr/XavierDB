//! ls — GET /ls: list the databases (and collections) the caller may read.
//!
//! Prerequisite: run setup_ls first (creates the app with GET on `*`).
//! Logs in as demo@xdb-ls and lists every database it can read, then asks
//! for the collections of the first database it saw. Both response shapes
//! are printed raw: flat db names, and {db, collections}.
//!
//! Usage:
//!   cargo run --manifest-path examples/Cargo.toml --bin ls -- \
//!       [--token <app-token>] [--app xdb-ls] [--name demo] [--base-url http://127.0.0.1:8000]

use serde_json::{json, Value};

fn main() {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(60))
        .build();
    let (base, app, name, token) = args();
    let jwt = login(&agent, &base, &format!("{name}@{app}"), &token);
    println!("authenticated as {name}@{app}\n");

    // flat list of databases the caller may GET
    let (status, body) = get(&agent, &base, &jwt, "/ls", &[]);
    println!("GET /ls -> {status}");
    println!(
        "{}",
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );

    // collections of the first visible database (always exists by
    // construction — the flat list just proved it)
    let dbs = body["databases"].as_array().cloned().unwrap_or_default();
    if let Some(first) = dbs.first().and_then(|d| d.as_str()) {
        println!("\nGET /ls?db={first}");
        let (status, body) = get(&agent, &base, &jwt, "/ls", &[("db", first)]);
        println!("-> {status}");
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    } else {
        println!("\nno databases visible — seed some data first (e.g. run the write example)");
    }
}

fn get(
    agent: &ureq::Agent,
    base: &str,
    jwt: &str,
    path: &str,
    params: &[(&str, &str)],
) -> (u16, Value) {
    let mut req = agent
        .get(&format!("{base}{path}"))
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
    let mut app = "xdb-ls".to_string();
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
