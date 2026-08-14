//! Meta-endpoint tests: /ls (flat, per-db, pagination, permission filtering,
//! limit validation), /health shape, and the error contract across routes.
//! All requests use cached JWTs (no fresh /auth).

mod common;

use common::*;
use serde_json::Value;

fn db_names(body: &Value) -> Vec<String> {
    body["databases"]
        .as_array()
        .expect("databases array")
        .iter()
        .filter_map(|d| d.as_str().map(str::to_string))
        .collect()
}

/// Percent-encode a path segment so invalid chars reach the server intact
/// (ureq rejects raw quotes/dollars in URLs).
fn pct_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn assert_error_contract(b: &Value, code: &str, status: u16) {
    let obj = b.as_object().expect("error body is an object");
    assert_eq!(obj.len(), 3, "exactly error/code/status: {b}");
    assert!(obj.contains_key("error"));
    assert_eq!(b["code"], code, "{b}");
    assert_eq!(b["status"], status, "{b}");
}

#[test]
fn ls_flat() {
    ensure_server();
    let agent = agent();
    let (s, b) = ls(&agent, &jwt("main"), &[]);
    assert_eq!(s, 200, "{b}");
    let dbs = db_names(&b);
    assert!(dbs.contains(&DB_SHARED.to_string()));
    assert!(dbs.contains(&DB_EXTRA.to_string()));
    assert!(dbs.contains(&DB_SECRET.to_string()));
    assert!(b["limit_applied"].is_number());
    assert_eq!(b["has_more"], false);
    assert!(b["next_cursor"].is_null(), "no pagination needed: {b}");
}

#[test]
fn ls_db_collections() {
    ensure_server();
    let agent = agent();
    let (s, b) = ls(&agent, &jwt("main"), &[("db", DB_SHARED)]);
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["db"], DB_SHARED);
    let colls: Vec<&str> = b["collections"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c.as_str())
        .collect();
    assert!(colls.contains(&"seed"), "collections: {colls:?}");
}

#[test]
fn ls_db_missing() {
    ensure_server();
    let agent = agent();
    let (s, b) = ls(&agent, &jwt("main"), &[("db", "no_such_db_xyz")]);
    assert_eq!(s, 404, "{b}");
    assert_error_contract(&b, "NOT_FOUND", 404);
}

#[test]
fn ls_db_forbidden() {
    ensure_server();
    let agent = agent();
    // xdb_tb_secret exists but ruser is denied GET on it
    let (s, b) = ls(&agent, &jwt("ruser"), &[("db", DB_SECRET)]);
    assert_eq!(s, 403, "{b}");
    assert_error_contract(&b, "FORBIDDEN", 403);
}

#[test]
fn ls_filtered_by_perms() {
    ensure_server();
    let agent = agent();
    // ruser: GET * except deny xdb_tb_secret
    let (s, b) = ls(&agent, &jwt("ruser"), &[]);
    assert_eq!(s, 200, "{b}");
    let dbs = db_names(&b);
    assert!(dbs.contains(&DB_SHARED.to_string()));
    assert!(!dbs.contains(&DB_SECRET.to_string()));
    // reader: GET only xdb_tb_shared
    let (s, b) = ls(&agent, &jwt("reader"), &[]);
    assert_eq!(s, 200, "{b}");
    let dbs = db_names(&b);
    assert!(dbs.contains(&DB_SHARED.to_string()));
    assert!(!dbs.contains(&DB_EXTRA.to_string()));
    assert!(!dbs.contains(&DB_SECRET.to_string()));
}

#[test]
fn ls_cursor_pagination() {
    ensure_server();
    let agent = agent();
    let jwt = jwt("main");

    // full unfiltered list (no params -> everything the caller may read)
    let (s, full) = ls(&agent, &jwt, &[]);
    assert_eq!(s, 200, "{full}");
    let full_set: std::collections::HashSet<String> = db_names(&full).into_iter().collect();
    assert!(
        full_set.len() >= 3,
        "machine must have >2 dbs (got {})",
        full_set.len()
    );

    // page through with limit=2
    let (s, page) = ls(&agent, &jwt, &[("limit", "2")]);
    assert_eq!(s, 200, "{page}");
    assert_eq!(page["has_more"], true, "many dbs -> pagination needed");
    let mut seen: Vec<String> = db_names(&page);
    assert_eq!(seen.len(), 2);
    let mut cursor = page["next_cursor"].as_str().map(str::to_string);
    let mut iterations = 1;
    while let Some(c) = cursor {
        iterations += 1;
        assert!(iterations <= 50, "cursor walk did not terminate");
        let (s, next) = ls(&agent, &jwt, &[("limit", "2"), ("cursor", &c)]);
        assert_eq!(s, 200, "{next}");
        seen.extend(db_names(&next));
        cursor = next["next_cursor"].as_str().map(str::to_string);
    }

    // no duplicates across pages
    let mut uniq: std::collections::HashSet<&String> = std::collections::HashSet::new();
    for d in &seen {
        assert!(uniq.insert(d), "duplicate db {d} across pages");
    }
    // union of pages == the full list
    let seen_set: std::collections::HashSet<String> = seen.into_iter().collect();
    assert_eq!(seen_set, full_set, "paginated union equals full /ls list");
}

#[test]
fn ls_limit_zero() {
    ensure_server();
    let agent = agent();
    let (s, b) = ls(&agent, &jwt("main"), &[("limit", "0")]);
    assert_eq!(s, 400, "{b}");
    assert_error_contract(&b, "INVALID_LIMIT", 400);
}

#[test]
fn health_shape() {
    ensure_server();
    let agent = agent();
    let (s, b) = health(&agent);
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["status"], "ok");
    assert_eq!(b["mongodb"]["reachable"], true);
    assert_eq!(b["app"]["status"], "ok");
    assert!(b["checked_at_ms"].is_number());
    assert!(b["next_refresh_seconds"].is_number());
    assert_eq!(b.as_object().unwrap().len() >= 3, true);
}

#[test]
fn error_contract_everywhere() {
    ensure_server();
    let agent = agent();
    let jwt = jwt("main");

    // dot in the DATABASE segment -> 400 BAD_REQUEST
    let (s, b) = get_q(&agent, &jwt, "xdb.tb_shared", "seed", &[]);
    assert_eq!(s, 400, "{b}");
    assert_error_contract(&b, "BAD_REQUEST", 400);

    // dot in the COLLECTION segment is valid (dotted coll names are allowed)
    let (s, b) = get_q(&agent, &jwt, DB_SHARED, "bad..name", &[]);
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["documents"].as_array().unwrap().len(), 0);

    // quote in collection -> 400
    let (s, b) = get(
        &agent,
        &format!("{}/q/{}/{}", base(), DB_SHARED, pct_encode("\"bad")),
        Some(&jwt),
    );
    assert_eq!(s, 400, "{b}");
    assert_error_contract(&b, "BAD_REQUEST", 400);

    // dollar in collection -> 400
    let (s, b) = get(
        &agent,
        &format!("{}/q/{}/{}", base(), DB_SHARED, pct_encode("a$b")),
        Some(&jwt),
    );
    assert_eq!(s, 400, "{b}");
    assert_error_contract(&b, "BAD_REQUEST", 400);

    // empty db segment -> 400
    let (s, b) = get_q(&agent, &jwt, "", "x", &[]);
    assert_eq!(s, 400, "{b}");
    assert_error_contract(&b, "BAD_REQUEST", 400);

    // nonexistent db -> 200 with an empty document list (not 404)
    let (s, b) = get_q(&agent, &jwt, "ok-db", "ok-coll", &[]);
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["documents"].as_array().unwrap().len(), 0);

    // malformed query value (non-numeric limit) -> 400 with the contract
    let (s, b) = get(
        &agent,
        &format!("{}/q/{}/seed?limit=abc", base(), DB_SHARED),
        Some(&jwt),
    );
    assert_eq!(s, 400, "{b}");
    assert_error_contract(&b, "BAD_REQUEST", 400);

    // invalid percent-encoding is decoded leniently, so the filter value
    // reaches the handler and fails as INVALID_FILTER (still the contract)
    let (s, b) = get(
        &agent,
        &format!("{}/q/{}/seed?filter=%zz", base(), DB_SHARED),
        Some(&jwt),
    );
    assert_eq!(s, 400, "{b}");
    assert_error_contract(&b, "INVALID_FILTER", 400);

    // unknown route -> 404 with the error contract
    let (s, b) = get(
        &agent,
        &format!("{}/totally/not/a/route", base()),
        Some(&jwt),
    );
    assert_eq!(s, 404, "{b}");
    assert_error_contract(&b, "NOT_FOUND", 404);
}
