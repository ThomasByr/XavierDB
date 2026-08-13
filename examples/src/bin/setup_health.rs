//! setup_health — grant the app used by the `health` showcase.
//!
//! Uses the dashboard API: login as admin, then POST /dashboard/api/perms to
//! create (or refresh) the `xdb-health` app with a shared token and GET
//! rights on db1. Note: GET /health itself is public and needs no token —
//! the app is created anyway so the setup/showcase pair stays uniform.
//! Safe to re-run: only this app is touched (merge semantics), its allow/
//! deny are replaced wholesale, and the token is rehashed on every run.
//!
//! Usage:
//!   cargo run --manifest-path examples/Cargo.toml --bin setup_health -- \
//!       --admin-pass <dashboard-password>
//! Options: --admin-user (env XDB_ADMIN_USER, default "admin"),
//!          --token     (env XDB_TOKEN, default "demo-token-change-me"),
//!          --base-url  (default http://127.0.0.1:8000)

use serde_json::json;

const APP: &str = "xdb-health";

fn main() {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(60))
        .build();
    let (base, admin_user, admin_pass, token) = args();

    let resp = agent
        .post(&format!("{base}/dashboard/api/login"))
        .send_json(json!({ "username": admin_user, "password": admin_pass }))
        .unwrap_or_else(|e| die(&format!("dashboard login failed: {e}")));
    let cookie = resp
        .header("set-cookie")
        .and_then(|c| c.split(';').next().map(str::to_string))
        .expect("login response has no Set-Cookie header");
    println!("logged in as {admin_user}");

    let resp = agent
        .post(&format!("{base}/dashboard/api/perms"))
        .set("Cookie", &cookie)
        .send_json(json!({
            "apps": [{
                "app": APP,
                "allow": [
                    { "actions": ["GET"], "databases": ["db1"], "collections": ["*"] }
                ],
                "deny": [],
                "set_token": token,
            }]
        }))
        .unwrap_or_else(|e| die(&format!("perms update failed: {e}")));
    let status = resp.status();
    let body: serde_json::Value = resp.into_json().unwrap_or_default();
    if status != 200 {
        die(&format!("perms update failed ({status}): {body}"));
    }
    println!("perms ok: app {APP}, token set, GET on db1");
}

fn args() -> (String, String, String, String) {
    let mut base = "http://127.0.0.1:8000".to_string();
    let mut admin_user = std::env::var("XDB_ADMIN_USER").unwrap_or_else(|_| "admin".into());
    let mut admin_pass = std::env::var("XDB_ADMIN_PASS").ok();
    let mut token = std::env::var("XDB_TOKEN").unwrap_or_else(|_| "demo-token-change-me".into());
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--base-url" => base = it.next().expect("--base-url needs a value"),
            "--admin-user" => admin_user = it.next().expect("--admin-user needs a value"),
            "--admin-pass" => admin_pass = Some(it.next().expect("--admin-pass needs a value")),
            "--token" => token = it.next().expect("--token needs a value"),
            other => die(&format!("unknown option {other}")),
        }
    }
    let admin_pass = admin_pass.unwrap_or_else(|| {
        die("missing dashboard password: pass --admin-pass or set XDB_ADMIN_PASS")
    });
    (base, admin_user, admin_pass, token)
}

fn die(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}
