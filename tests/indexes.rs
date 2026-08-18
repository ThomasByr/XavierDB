//! Index endpoint tests: /q/{db}/{coll}/indexes (list / ensure / drop),
//! the ensure decision table (create vs exists vs conflict), validation,
//! and the INDEX permission model (GET lists, INDEX manages).

mod common;

use common::*;
use serde_json::{Value, json};

fn uniq(prefix: &str) -> String {
    format!("{prefix}_{}", std::process::id())
}

fn get_idx(agent: &ureq::Agent, jwt: &str, db: &str, coll: &str) -> (u16, Value) {
    get(agent, &format!("{}/q/{db}/{coll}/indexes", base()), Some(jwt))
}

fn ensure_idx(
    agent: &ureq::Agent,
    jwt: &str,
    db: &str,
    coll: &str,
    body: &Value,
) -> (u16, Value) {
    post(
        agent,
        &format!("{}/q/{db}/{coll}/indexes", base()),
        Some(jwt),
        None,
        Some(body),
    )
}

fn drop_idx(agent: &ureq::Agent, jwt: &str, db: &str, coll: &str, name: &str) -> (u16, Value) {
    call(
        agent,
        "DELETE",
        &format!("{}/q/{db}/{coll}/indexes", base()),
        Some(jwt),
        None,
        Some(&json!({ "name": name })),
    )
}

fn names(body: &Value) -> Vec<String> {
    body["indexes"]
        .as_array()
        .expect("indexes array")
        .iter()
        .map(|i| i["name"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn find_index<'a>(body: &'a Value, name: &str) -> Option<&'a Value> {
    body["indexes"]
        .as_array()?
        .iter()
        .find(|i| i["name"] == name)
}

#[test]
fn ensure_lifecycle() {
    ensure_server();
    let agent = agent();
    let jwt = jwt("main");
    let coll = uniq("idx_life");

    // fresh collection -> create, 201
    let (s, b) = ensure_idx(
        &agent,
        &jwt,
        DB_SHARED,
        &coll,
        &json!({ "keys": { "customer": 1, "at": -1 }, "name": "cust_at" }),
    );
    assert_eq!(s, 201, "{b}");
    assert_eq!(b["created"], true);
    assert_eq!(b["name"], "cust_at");

    // same body -> idempotent no-op, 200
    let (s, b) = ensure_idx(
        &agent,
        &jwt,
        DB_SHARED,
        &coll,
        &json!({ "keys": { "customer": 1, "at": -1 }, "name": "cust_at" }),
    );
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["created"], false);
    assert_eq!(b["name"], "cust_at");

    // same keys, no explicit name -> still "exists"
    let (s, b) = ensure_idx(&agent, &jwt, DB_SHARED, &coll, &json!({ "keys": { "customer": 1, "at": -1 } }));
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["created"], false);
    assert_eq!(b["name"], "cust_at");
}

#[test]
fn ensure_conflicts() {
    ensure_server();
    let agent = agent();
    let jwt = jwt("main");
    let coll = uniq("idx_conf");

    let _ = ensure_idx(&agent, &jwt, DB_SHARED, &coll, &json!({ "keys": { "a": 1 }, "name": "a_idx" }));

    // same name, different keys -> 409
    let (s, b) = ensure_idx(&agent, &jwt, DB_SHARED, &coll, &json!({ "keys": { "b": 1 }, "name": "a_idx" }));
    assert_eq!(s, 409, "{b}");
    assert_eq!(err_code(&b), "CONFLICT");

    // same keys, different options -> 409
    let (s, b) = ensure_idx(&agent, &jwt, DB_SHARED, &coll, &json!({ "keys": { "a": 1 }, "unique": true }));
    assert_eq!(s, 409, "{b}");
    assert_eq!(err_code(&b), "CONFLICT");
}

#[test]
fn ensure_ttl_and_options() {
    ensure_server();
    let agent = agent();
    let jwt = jwt("main");
    let coll = uniq("idx_ttl");

    let (s, b) = ensure_idx(
        &agent,
        &jwt,
        DB_SHARED,
        &coll,
        &json!({ "keys": { "created": 1 }, "name": "ttl", "expire_after_seconds": 3600 }),
    );
    assert_eq!(s, 201, "{b}");

    // same TTL -> exists
    let (s, b) = ensure_idx(
        &agent,
        &jwt,
        DB_SHARED,
        &coll,
        &json!({ "keys": { "created": 1 }, "expire_after_seconds": 3600 }),
    );
    assert_eq!(s, 200, "{b}");

    // different TTL -> conflict (changing a TTL needs collMod, refuse loudly)
    let (s, b) = ensure_idx(
        &agent,
        &jwt,
        DB_SHARED,
        &coll,
        &json!({ "keys": { "created": 1 }, "expire_after_seconds": 7200 }),
    );
    assert_eq!(s, 409, "{b}");

    // partial filter round-trips through the listing
    let (s, _) = ensure_idx(
        &agent,
        &jwt,
        DB_SHARED,
        &coll,
        &json!({ "keys": { "status": 1 }, "name": "part", "partial_filter_expression": { "status": "active" } }),
    );
    assert_eq!(s, 201);
    let (s, b) = get_idx(&agent, &jwt, DB_SHARED, &coll);
    assert_eq!(s, 200, "{b}");
    let part = find_index(&b, "part").expect("part index listed");
    assert_eq!(part["partial_filter_expression"], json!({ "status": "active" }));
    let ttl = find_index(&b, "ttl").expect("ttl index listed");
    assert_eq!(ttl["expire_after_seconds"], json!(3600));
}

#[test]
fn ensure_validation() {
    ensure_server();
    let agent = agent();
    let jwt = jwt("main");
    let coll = uniq("idx_valid");

    // bad direction
    let (s, b) = ensure_idx(&agent, &jwt, DB_SHARED, &coll, &json!({ "keys": { "a": 2 } }));
    assert_eq!(s, 400, "{b}");
    // empty keys
    let (s, b) = ensure_idx(&agent, &jwt, DB_SHARED, &coll, &json!({ "keys": {} }));
    assert_eq!(s, 400, "{b}");
    // keys not an object
    let (s, b) = ensure_idx(&agent, &jwt, DB_SHARED, &coll, &json!({ "keys": [1, 2] }));
    assert_eq!(s, 400, "{b}");
    // string direction is a valid index type (text)
    let (s, b) = ensure_idx(&agent, &jwt, DB_SHARED, &coll, &json!({ "keys": { "note": "text" } }));
    assert_eq!(s, 201, "{b}");
}

#[test]
fn list_shape() {
    ensure_server();
    let agent = agent();
    let jwt = jwt("main");
    // seeded collection -> _id_ index present
    let (s, b) = get_idx(&agent, &jwt, DB_SHARED, "seed");
    assert_eq!(s, 200, "{b}");
    let ns = names(&b);
    assert!(ns.contains(&"_id_".to_string()), "{ns:?}");
    assert_eq!(b["count"], json!(ns.len()));
    let id = find_index(&b, "_id_").unwrap();
    assert_eq!(id["keys"], json!({ "_id": 1 }));

    // missing collection -> 404
    let (s, b) = get_idx(&agent, &jwt, DB_SHARED, &uniq("idx_nope"));
    assert_eq!(s, 404, "{b}");
    assert_eq!(err_code(&b), "NOT_FOUND");
}

#[test]
fn drop_flow() {
    ensure_server();
    let agent = agent();
    let jwt = jwt("main");
    let coll = uniq("idx_drop");

    let _ = ensure_idx(&agent, &jwt, DB_SHARED, &coll, &json!({ "keys": { "a": 1 }, "name": "droppable" }));

    let (s, b) = drop_idx(&agent, &jwt, DB_SHARED, &coll, "droppable");
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["deleted"], true);

    // gone from the listing
    let (_, b) = get_idx(&agent, &jwt, DB_SHARED, &coll);
    assert!(!names(&b).contains(&"droppable".to_string()), "{b}");

    // unknown name -> 404
    let (s, b) = drop_idx(&agent, &jwt, DB_SHARED, &coll, "droppable");
    assert_eq!(s, 404, "{b}");
    assert_eq!(err_code(&b), "NOT_FOUND");

    // _id_ is protected
    let (s, b) = drop_idx(&agent, &jwt, DB_SHARED, &coll, "_id_");
    assert_eq!(s, 400, "{b}");

    // empty name -> 400
    let (s, b) = drop_idx(&agent, &jwt, DB_SHARED, &coll, "");
    assert_eq!(s, 400, "{b}");
}

#[test]
fn index_permissions() {
    ensure_server();
    let agent = agent();
    let main = jwt("main");
    let coll = uniq("idx_perm");
    let _ = ensure_idx(&agent, &main, DB_SHARED, &coll, &json!({ "keys": { "a": 1 } }));

    // GET-only app: can LIST (read implies seeing indexes)...
    let reader = jwt("reader");
    let (s, b) = get_idx(&agent, &reader, DB_SHARED, &coll);
    assert_eq!(s, 200, "{b}");

    // ...but not ensure or drop (INDEX is a separate, default-deny action)
    let (s, b) = ensure_idx(&agent, &reader, DB_SHARED, &coll, &json!({ "keys": { "b": 1 } }));
    assert_eq!(s, 403, "{b}");
    assert_eq!(err_code(&b), "FORBIDDEN");
    let (s, b) = drop_idx(&agent, &reader, DB_SHARED, &coll, "a_1");
    assert_eq!(s, 403, "{b}");
    assert_eq!(err_code(&b), "FORBIDDEN");
}

#[test]
fn index_error_contract() {
    ensure_server();
    let agent = agent();
    let _jwt = jwt("main");
    // unauthenticated -> 401 standard body
    let (s, b) = get(&agent, &format!("{}/q/{DB_SHARED}/seed/indexes", base()), None);
    assert_eq!(s, 401, "{b}");
    assert_eq!(err_code(&b), "UNAUTHORIZED");
    assert_eq!(err_status(&b), 401);
}
