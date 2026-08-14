//! Permission matrix tests — the layered first-match-wins evaluation
//! (name.deny -> name.allow -> app.deny -> app.allow -> deny) exercised
//! black-box through /q, plus glob scoping and the 403 error contract.
//! All requests use cached JWTs (no fresh /auth).

mod common;

use common::*;
use serde_json::json;

fn uniq(prefix: &str) -> String {
    format!("{prefix}_{}", std::process::id())
}

#[test]
fn ruser_glob_read() {
    ensure_server();
    let agent = agent();
    let jwt = jwt("ruser");
    // explicit db
    let (s, b) = get_q(&agent, &jwt, DB_SHARED, "seed", &[]);
    assert_eq!(s, 200, "{b}");
    assert!(
        b["documents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["_id"] == "seed-1")
    );
    // '*' glob on databases matches machine dbs (db1 exists)
    let (s, b) = get_q(&agent, &jwt, "db1", "items", &[]);
    assert_eq!(s, 200, "{b}");
    // another xdb_tb_* db
    let (s, b) = get_q(&agent, &jwt, DB_EXTRA, "seed", &[]);
    assert_eq!(s, 200, "{b}");
}

#[test]
fn ruser_deny_beats_allow() {
    ensure_server();
    let agent = agent();
    let (s, b) = get_q(&agent, &jwt("ruser"), DB_SECRET, "seed", &[]);
    assert_eq!(s, 403, "{b}");
    assert_eq!(b["code"], "FORBIDDEN");
    assert_eq!(b["status"], 403);
}

#[test]
fn ruser_write_denied() {
    ensure_server();
    let agent = agent();
    let coll = uniq("pm_rw");
    let (s, b) = post_q(
        &agent,
        &jwt("ruser"),
        DB_SHARED,
        &coll,
        &json!({ "data": { "v": 1 } }),
    );
    assert_eq!(s, 403, "{b}");
    assert_eq!(b["code"], "FORBIDDEN");
    // nothing was inserted (verify as the full-access identity)
    let (s, b) = get_q(&agent, &jwt("main"), DB_SHARED, &coll, &[]);
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["documents"].as_array().unwrap().len(), 0);
}

#[test]
fn reader_scoped() {
    ensure_server();
    let agent = agent();
    let jwt = jwt("reader");
    // allowed db
    let (s, b) = get_q(&agent, &jwt, DB_SHARED, "seed", &[]);
    assert_eq!(s, 200, "{b}");
    // other db -> 403
    let (s, b) = get_q(&agent, &jwt, DB_EXTRA, "seed", &[]);
    assert_eq!(s, 403, "{b}");
    assert_eq!(b["code"], "FORBIDDEN");
    // no write rule at all
    let coll = uniq("pm_rd");
    let (s, b) = post_q(
        &agent,
        &jwt,
        DB_SHARED,
        &coll,
        &json!({ "data": { "v": 1 } }),
    );
    assert_eq!(s, 403, "{b}");
    assert_eq!(b["code"], "FORBIDDEN");
}

#[test]
fn m1_glob_and_deny() {
    ensure_server();
    let agent = agent();
    let jwt = jwt("m1user");
    // app allow glob xdb_tb_* matches xdb_tb_extra
    let (s, b) = get_q(&agent, &jwt, DB_EXTRA, "seed", &[]);
    assert_eq!(s, 200, "{b}");
    // app deny GET xdb_tb_secret wins over the glob
    let (s, b) = get_q(&agent, &jwt, DB_SECRET, "seed", &[]);
    assert_eq!(s, 403, "{b}");
    assert_eq!(b["code"], "FORBIDDEN");
    // db1 does not match xdb_tb_*
    let (s, b) = get_q(&agent, &jwt, "db1", "items", &[]);
    assert_eq!(s, 403, "{b}");
    assert_eq!(b["code"], "FORBIDDEN");
}

#[test]
fn m1_name_deny_beats_app_allow() {
    ensure_server();
    let agent = agent();
    let main = jwt("main");
    let coll = uniq("pm_m1del");
    let id = "del-me";
    seed(&agent, &main, DB_SHARED, &coll, id, json!({ "v": 1 }));

    // m1user has a NAME-level deny DELETE on xdb_tb_shared -> 403 even
    // though the app allows DELETE on xdb_tb_shared
    let (s, b) = delete_q(
        &agent,
        &jwt("m1user"),
        DB_SHARED,
        &coll,
        &json!({ "filter": { "_id": id } }),
    );
    assert_eq!(s, 403, "{b}");
    assert_eq!(b["code"], "FORBIDDEN");

    // m1user2 has no name rules -> app allow wins -> delete succeeds
    let (s, b) = delete_q(
        &agent,
        &jwt("m1user2"),
        DB_SHARED,
        &coll,
        &json!({ "filter": { "_id": id } }),
    );
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["deleted_count"], 1);

    // re-seed so the fixture world stays idempotent
    seed(&agent, &main, DB_SHARED, &coll, id, json!({ "v": 1 }));
}

#[test]
fn u2_ingester() {
    ensure_server();
    let agent = agent();
    let jwt = jwt("u2");
    let coll = uniq("pm_u2");
    // POST insert -> 201
    let (s, b) = post_q(
        &agent,
        &jwt,
        DB_SHARED,
        &coll,
        &json!({ "data": { "_id": "ing-1", "v": 1 } }),
    );
    assert_eq!(s, 201, "{b}");
    assert_eq!(b["inserted_count"], 1);
    // PATCH upsert -> 201 (new doc)
    let (s, b) = patch_q(
        &agent,
        &jwt,
        DB_SHARED,
        &coll,
        &json!({ "filter": { "_id": "ing-2" }, "data": { "v": 2 } }),
    );
    assert_eq!(s, 201, "{b}");
    assert_eq!(b["upserted"], true);
    // everything else is denied
    let (s, b) = get_q(&agent, &jwt, DB_SHARED, &coll, &[]);
    assert_eq!(s, 403, "{b}");
    assert_eq!(b["code"], "FORBIDDEN");
    let (s, b) = put_q(
        &agent,
        &jwt,
        DB_SHARED,
        &coll,
        &json!({ "filter": { "_id": "ing-1" }, "data": { "v": 3 } }),
    );
    assert_eq!(s, 403, "{b}");
    let (s, b) = delete_q(
        &agent,
        &jwt,
        DB_SHARED,
        &coll,
        &json!({ "filter": { "_id": "ing-1" } }),
    );
    assert_eq!(s, 403, "{b}");
}

#[test]
fn u3_collection_scope() {
    ensure_server();
    let agent = agent();
    let main = jwt("main");
    // the fixture rule for u3 is scoped to the literal collection "public"
    seed(
        &agent,
        &main,
        DB_SHARED,
        "public",
        "pub-1",
        json!({ "v": 1 }),
    );
    seed(
        &agent,
        &main,
        DB_SHARED,
        "other",
        "oth-1",
        json!({ "v": 1 }),
    );

    let u3 = jwt("u3");
    // allowed: xdb_tb_shared / "public"
    let (s, b) = get_q(&agent, &u3, DB_SHARED, "public", &[]);
    assert_eq!(s, 200, "{b}");
    assert!(
        b["documents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["_id"] == "pub-1")
    );
    // denied: another collection in the same db
    let (s, b) = get_q(&agent, &u3, DB_SHARED, "other", &[]);
    assert_eq!(s, 403, "{b}");
    assert_eq!(b["code"], "FORBIDDEN");
    // denied: another database
    let (s, b) = get_q(&agent, &u3, DB_EXTRA, "seed", &[]);
    assert_eq!(s, 403, "{b}");
    assert_eq!(b["code"], "FORBIDDEN");
}

#[test]
fn error_contract_403() {
    ensure_server();
    let agent = agent();
    let (s, b) = get_q(&agent, &jwt("ruser"), DB_SECRET, "seed", &[]);
    assert_eq!(s, 403);
    let obj = b.as_object().expect("error body is an object");
    assert_eq!(obj.len(), 3, "exactly error/code/status: {b}");
    assert!(obj.contains_key("error"));
    assert_eq!(b["code"], "FORBIDDEN");
    assert_eq!(b["status"], 403);
}
