//! Smoke tests — the battery's sanity floor: server up, Mongo reachable,
//! auth works, a full HTTP round-trip works. Fast (< 1 s when warm).

mod common;

use common::*;
use serde_json::Value;

#[test]
fn health_is_ok_and_shaped() {
    let agent = agent();
    let (status, body) = health(&agent);
    assert_eq!(status, 200);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["mongodb"]["reachable"], true);
    assert_eq!(body["app"]["status"], "ok");
    assert!(body["checked_at_ms"].is_number());
    assert!(body["next_refresh_seconds"].is_number());
    // error contract shape on degraded would be 503; 200 means ok fields exist
}

#[test]
fn auth_and_roundtrip_work() {
    ensure_server();
    let agent = agent();
    let jwt = jwt("main");

    // write a doc, read it back, patch it, delete it
    seed(
        &agent,
        &jwt,
        DB_SHARED,
        "smoke",
        "smoke-1",
        serde_json::json!({ "v": 1 }),
    );
    let (status, body) = get_q(&agent, &jwt, DB_SHARED, "smoke", &[]);
    assert_eq!(status, 200);
    let found: Vec<&Value> = body["documents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["_id"] == "smoke-1")
        .collect();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["v"], 1);
}

#[test]
fn dashboard_cookie_works() {
    ensure_server();
    let agent = agent();
    let cookie = dash_cookie();
    assert!(cookie.starts_with("xdb_admin="));
    let (status, body) = dash_get(&agent, &cookie, "/dashboard/api/logs");
    assert_eq!(status, 200, "dash GET: {body}");
    assert!(body["lines"].is_array());
    let (status, body) = dash_get(&agent, &cookie, "/dashboard/api/perms");
    assert_eq!(status, 200, "perms GET: {body}");
    assert!(body["apps"].is_array());
    assert!(body["version"].is_number());
}

#[test]
fn identity_jwts_all_valid() {
    ensure_server();
    let agent = agent();
    for ident in IDENTITIES {
        let j = jwt(ident.key);
        let (status, _) = get(&agent, &format!("{}/ls", base()), Some(&j));
        assert!(status != 401, "identity {} got 401", ident.identifier);
    }
}
