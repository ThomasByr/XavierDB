//! Keyset cursor pagination over GET /q — full walks, deterministic no-sort
//! order, filtered walks, invalid/tampered cursors, the array-sort guard and
//! NaN/missing boundary behavior. Every test seeds its own collection
//! (db xdb_tb_page).

mod common;

use common::*;
use serde_json::{Value, json};

const DB: &str = "xdb_tb_page";

/// Seed `count` docs {n: i} with zero-padded _ids "d000".. (idempotent).
fn seed_n(agent: &ureq::Agent, jwt: &str, coll: &str, count: usize) {
    for i in 0..count {
        seed(
            agent,
            jwt,
            DB,
            coll,
            &format!("d{i:03}"),
            json!({ "n": i as i64 }),
        );
    }
}

fn ids_of(docs: &[Value]) -> Vec<String> {
    docs.iter()
        .map(|d| d["_id"].as_str().unwrap().to_string())
        .collect()
}

/// Walk the cursor to the end. `params` are the per-page params minus
/// `cursor`; returns the concatenated documents in page order.
fn walk(agent: &ureq::Agent, jwt: &str, coll: &str, params: &[(&str, &str)]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut page = params.to_vec();
        if let Some(c) = &cursor {
            page.push(("cursor", c));
        }
        let (s, b) = get_q(agent, jwt, DB, coll, &page);
        assert_eq!(s, 200, "{b}");
        out.extend(b["documents"].as_array().unwrap().iter().cloned());
        if b["has_more"] == false {
            return out;
        }
        cursor = b["next_cursor"].as_str().map(|s| s.to_string());
    }
}

#[test]
fn walk_full_set_250() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "pg_walk_full_set_250";
    seed_n(&agent, &jwt, coll, 250);

    // first page: capped at the enforced limit, cursor available
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("sort", r#"{"n":1}"#), ("limit", "200")],
    );
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["has_more"], true);
    assert!(b["next_cursor"].is_string());
    assert_eq!(b["count"], b["limit_applied"]);
    assert!(b["limit_applied"].as_u64().unwrap() <= 200);

    let docs = walk(
        &agent,
        &jwt,
        coll,
        &[("sort", r#"{"n":1}"#), ("limit", "200")],
    );
    assert_eq!(docs.len(), 250);
    let ids = ids_of(&docs);
    let mut uniq = ids.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), 250);
    // strictly ascending n across page boundaries
    let ns: Vec<i64> = docs.iter().map(|d| d["n"].as_i64().unwrap()).collect();
    assert_eq!(ns.len(), 250);
    for w in ns.windows(2) {
        assert!(w[0] < w[1], "order broken at {} vs {}", w[0], w[1]);
    }
}

#[test]
fn sort_desc_walk() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "pg_sort_desc_walk";
    seed_n(&agent, &jwt, coll, 250);

    let docs = walk(
        &agent,
        &jwt,
        coll,
        &[("sort", r#"{"n":-1}"#), ("limit", "200")],
    );
    assert_eq!(docs.len(), 250);
    let mut uniq = ids_of(&docs);
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), 250);
    let ns: Vec<i64> = docs.iter().map(|d| d["n"].as_i64().unwrap()).collect();
    for w in ns.windows(2) {
        assert!(w[0] > w[1], "order broken at {} vs {}", w[0], w[1]);
    }
}

#[test]
fn no_sort_paginates_by_id() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "pg_no_sort_paginates_by_id";
    seed_n(&agent, &jwt, coll, 30);

    // no sort: normalize_sort appends _id:1 -> deterministic _id-asc walk
    let docs = walk(&agent, &jwt, coll, &[("limit", "10")]);
    assert_eq!(docs.len(), 30);
    let ids = ids_of(&docs);
    assert_eq!(ids.len(), 30);
    // no duplicates and globally ascending (zero-padded _ids sort lexically)
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
    let mut uniq = sorted.clone();
    uniq.dedup();
    assert_eq!(uniq.len(), 30);
}

#[test]
fn filter_pagination() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "pg_filter_pagination";
    for i in 0..40 {
        let group = if i % 2 == 0 { "a" } else { "b" };
        seed(
            &agent,
            &jwt,
            DB,
            coll,
            &format!("d{i:03}"),
            json!({ "n": i as i64, "group": group }),
        );
    }

    let docs = walk(
        &agent,
        &jwt,
        coll,
        &[
            ("filter", r#"{"group":"a"}"#),
            ("sort", r#"{"n":1}"#),
            ("limit", "5"),
        ],
    );
    // every returned doc is in group a, exactly the 20 even-n docs
    assert_eq!(docs.len(), 20);
    assert!(docs.iter().all(|d| d["group"] == "a"));
    let mut uniq = ids_of(&docs);
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), 20);
    let ns: Vec<i64> = docs.iter().map(|d| d["n"].as_i64().unwrap()).collect();
    for w in ns.windows(2) {
        assert!(w[0] < w[1]);
    }
}

#[test]
fn invalid_cursors() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "pg_invalid_cursors";
    seed_n(&agent, &jwt, coll, 20);

    // garbage
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("cursor", "garbage")]);
    assert_eq!(s, 400, "{b}");
    assert_eq!(err_code(&b), "INVALID_CURSOR");

    // a real cursor used on a different collection
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("sort", r#"{"n":1}"#), ("limit", "5")],
    );
    assert_eq!(s, 200, "{b}");
    let cur = b["next_cursor"].as_str().unwrap().to_string();
    let other = "pg_invalid_cursors_b";
    let (s, b) = get_q(&agent, &jwt, DB, other, &[("cursor", &cur)]);
    assert_eq!(s, 400, "{b}");
    assert_eq!(err_code(&b), "INVALID_CURSOR");

    // a cursor with a different sort than requested
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("sort", r#"{"n":-1}"#), ("cursor", &cur)],
    );
    assert_eq!(s, 400, "{b}");
    assert_eq!(err_code(&b), "INVALID_CURSOR");
}

#[test]
fn tampered_cursor() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "pg_tampered_cursor";
    seed_n(&agent, &jwt, coll, 20);

    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("sort", r#"{"n":1}"#), ("limit", "5")],
    );
    assert_eq!(s, 200, "{b}");
    let cur = b["next_cursor"].as_str().unwrap().to_string();
    assert!(!cur.is_empty());

    // flip the last character: base64url decoding then fails or yields
    // corrupted JSON -> 400 INVALID_CURSOR
    let mut tampered = cur;
    let last = tampered.pop().unwrap();
    let flipped = if last == 'A' { 'B' } else { 'A' };
    tampered.push(flipped);

    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("cursor", &tampered)]);
    assert_eq!(s, 400, "{b}");
    assert_eq!(err_code(&b), "INVALID_CURSOR");
}

#[test]
fn empty_results() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "pg_empty_results";
    seed_n(&agent, &jwt, coll, 5);

    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("filter", r#"{"n":{"$gt":100}}"#)],
    );
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["documents"], json!([]));
    assert_eq!(b["has_more"], false);
    assert!(b["next_cursor"].is_null());
    assert_eq!(b["count"], 0);
}

#[test]
fn array_sort_guard() {
    let agent = agent();
    let jwt = jwt("main");

    // case A: 3 scalar tags + 2 array tags; arrays sort after strings
    // ascending, so page 1 is fine and the continuation hits the array
    let coll = "pg_array_sort_guard";
    for (id, tags) in [
        ("s1", json!("t1")),
        ("s2", json!("t2")),
        ("s3", json!("t3")),
        ("a1", json!(["x"])),
        ("a2", json!(["y"])),
    ] {
        seed(&agent, &jwt, DB, coll, id, json!({ "tags": tags }));
    }
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("sort", r#"{"tags":1}"#), ("limit", "2")],
    );
    assert_eq!(s, 200, "{b}");
    let cur = b["next_cursor"].as_str().unwrap().to_string();
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("sort", r#"{"tags":1}"#), ("limit", "2"), ("cursor", &cur)],
    );
    assert_eq!(s, 400, "{b}");
    assert_eq!(err_code(&b), "BAD_REQUEST");

    // case B: array docs are only reachable on later pages. The guard
    // fires on the page whose boundary doc is an array AND more data
    // follows (has_more) — a final-page array is served without error.
    let coll = "pg_array_sort_guard_b";
    for i in 1..=5 {
        seed(
            &agent,
            &jwt,
            DB,
            coll,
            &format!("e{i}"),
            json!({ "tags": format!("t{i}") }),
        );
    }
    seed(
        &agent,
        &jwt,
        DB,
        coll,
        "e6",
        json!({ "tags": json!(["x"]) }),
    );
    seed(
        &agent,
        &jwt,
        DB,
        coll,
        "e7",
        json!({ "tags": json!(["y"]) }),
    );
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("sort", r#"{"tags":1}"#), ("limit", "2")],
    );
    assert_eq!(s, 200, "{b}");
    let c1 = b["next_cursor"].as_str().unwrap().to_string();
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("sort", r#"{"tags":1}"#), ("limit", "2"), ("cursor", &c1)],
    );
    assert_eq!(s, 200, "{b}");
    let c2 = b["next_cursor"].as_str().unwrap().to_string();
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("sort", r#"{"tags":1}"#), ("limit", "2"), ("cursor", &c2)],
    );
    assert_eq!(s, 400, "{b}");
    assert_eq!(err_code(&b), "BAD_REQUEST");
}

#[test]
fn nan_sort_paginates() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "pg_nan_sort_paginates";
    seed(
        &agent,
        &jwt,
        DB,
        coll,
        "p1",
        json!({ "price": {"$numberDouble": "NaN"} }),
    );
    seed(&agent, &jwt, DB, coll, "p2", json!({ "price": 1 }));
    seed(&agent, &jwt, DB, coll, "p3", json!({ "price": 5 }));
    seed(&agent, &jwt, DB, coll, "p4", json!({ "k": 1 })); // missing price
    seed(&agent, &jwt, DB, coll, "p5", json!({ "price": -3 }));

    // Mongo 8 ascending: missing (null) < NaN < numbers. NaN paginates
    // via the $gt:-Inf continuation branch, so the walk must be lossless.
    let docs = walk(
        &agent,
        &jwt,
        coll,
        &[("sort", r#"{"price":1}"#), ("limit", "2")],
    );
    assert_eq!(ids_of(&docs), vec!["p4", "p1", "p5", "p2", "p3"]);
    assert!(!docs[0].as_object().unwrap().contains_key("price"));
    assert_eq!(docs[1]["price"], json!({"$numberDouble": "NaN"}));
    assert_eq!(docs[2]["price"], -3);
    assert_eq!(docs[3]["price"], 1);
    assert_eq!(docs[4]["price"], 5);
}

#[test]
fn limit_cap_and_continuation() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "pg_limit_cap_and_continuation";
    seed_n(&agent, &jwt, coll, 10);

    // a huge limit caps at the enforced adaptive limit; all 10 fit, so the
    // response is truncated but has no cursor
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("limit", "500")]);
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["truncated"], true);
    assert!(b["limit_applied"].as_u64().unwrap() <= 200);
    assert_eq!(b["count"], 10);
    assert_eq!(b["has_more"], false);
    assert!(b["next_cursor"].is_null());

    // the same set still walks completely with small pages
    let docs = walk(&agent, &jwt, coll, &[("limit", "3")]);
    assert_eq!(docs.len(), 10);
    let mut uniq = ids_of(&docs);
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), 10);
}
