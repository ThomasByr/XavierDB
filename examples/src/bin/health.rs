//! health — GET /health: the public, cached health document.
//!
//! Prerequisite: run setup_health first (optional — /health is public and
//! needs no token at all; the showcase deliberately does NOT authenticate).
//! Fetches the health document twice and prints it: the doc is cached for
//! `config.health.ttl_seconds` (default 5s), so two quick calls return the
//! same cached document (identical checked_at_ms) rather than two live
//! computations. 200 with status "ok" — 503 otherwise.
//!
//! Usage:
//!   cargo run --manifest-path examples/Cargo.toml --bin health -- \
//!       [--base-url http://127.0.0.1:8000]

use serde_json::Value;

fn main() {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(60))
        .build();
    let base = args();

    let (status, body) = get(&agent, &base, "/health");
    println!("1. GET /health (no token) -> {status}");
    println!(
        "{}",
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );

    // the doc is cached: an immediate second call returns the same document
    std::thread::sleep(std::time::Duration::from_millis(200));
    let (status, body2) = get(&agent, &base, "/health");
    let same = body["checked_at_ms"] == body2["checked_at_ms"];
    println!("2. GET /health again -> {status}");
    println!(
        "   same cached document: {same} (checked_at_ms {})",
        body2["checked_at_ms"]
    );
    println!(
        "   status: {} | mongodb: {} | app: {}",
        body2["status"], body2["mongodb"]["reachable"], body2["app"]["status"]
    );
}

fn get(agent: &ureq::Agent, base: &str, path: &str) -> (u16, Value) {
    finish(agent.get(&format!("{base}{path}")).call())
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

fn args() -> String {
    let mut base = "http://127.0.0.1:8000".to_string();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--base-url" => base = it.next().expect("--base-url needs a value"),
            other => die(&format!("unknown option {other}")),
        }
    }
    base
}

fn die(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}
