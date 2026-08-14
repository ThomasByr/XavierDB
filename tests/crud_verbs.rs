//! CRUD verb semantics for /q/{db}/{coll}: insert vs update-many on POST,
//! PUT 404 semantics, PATCH upsert, DELETE counting, body validation and the
//! write permission gate. All tests live in xdb_tb_crud (one xdb_tb_shared
//! gate check) and use their own collection.

mod common;

use common::*;
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

const DB_CRUD: &str = "xdb_tb_crud";

fn get_docs(agent: &ureq::Agent, jwt: &str, db: &str, coll: &str) -> Vec<Value> {
    let (status, body) = get_q(agent, jwt, db, coll, &[]);
    assert_eq!(status, 200, "GET {db}/{coll}: {body}");
    body["documents"]
        .as_array()
        .expect("documents array")
        .clone()
}

fn get_filtered(
    agent: &ureq::Agent,
    jwt: &str,
    db: &str,
    coll: &str,
    filter: &Value,
) -> (u16, Value) {
    let f = serde_json::to_string(filter).expect("filter json");
    get_q(agent, jwt, db, coll, &[("filter", &f)])
}

#[test]
fn insert_201_and_read_back() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "crud_insert_read";
    clear_coll(&agent, &jwt, DB_CRUD, coll);

    let (status, body) = post_q(
        &agent,
        &jwt,
        DB_CRUD,
        coll,
        &json!({"data": {"_id": "c1", "v": 1}}),
    );
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["inserted_count"], 1);
    assert_eq!(body["inserted_id"], "c1");

    let (status, body) = get_filtered(&agent, &jwt, DB_CRUD, coll, &json!({"_id": "c1"}));
    assert_eq!(status, 200, "{body}");
    // full response shape
    assert!(body.get("documents").is_some());
    assert!(body.get("next_cursor").is_some());
    assert!(body.get("has_more").is_some());
    assert!(body.get("truncated").is_some());
    assert!(body.get("limit_applied").is_some());
    assert!(body.get("count").is_some());
    let la = body["limit_applied"].as_u64().expect("limit_applied");
    assert!((1..=200).contains(&la), "limit_applied {la} out of range");
    assert_eq!(body["count"], 1);
    let docs = body["documents"].as_array().unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["_id"], "c1");
    assert_eq!(docs[0]["v"], 1);
}

#[test]
fn insert_generates_objectid() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "crud_insert_oid";
    clear_coll(&agent, &jwt, DB_CRUD, coll);

    let (status, body) = post_q(&agent, &jwt, DB_CRUD, coll, &json!({"data": {"name": "x"}}));
    assert_eq!(status, 201, "{body}");
    let id = body["inserted_id"]
        .as_str()
        .expect("inserted_id is a hex string")
        .to_string();
    assert_eq!(id.len(), 24);
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()), "bad hex: {id}");

    let (status, body) = get_filtered(&agent, &jwt, DB_CRUD, coll, &json!({"name": "x"}));
    assert_eq!(status, 200, "{body}");
    let docs = body["documents"].as_array().unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["_id"].as_str().unwrap(), id);
    assert_eq!(docs[0]["name"], "x");
}

#[test]
fn duplicate_id_conflict() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "crud_dup_id";
    clear_coll(&agent, &jwt, DB_CRUD, coll);

    let (status, body) = post_q(
        &agent,
        &jwt,
        DB_CRUD,
        coll,
        &json!({"data": {"_id": "dup1", "v": 1}}),
    );
    assert_eq!(status, 201, "{body}");
    let (status, body) = post_q(
        &agent,
        &jwt,
        DB_CRUD,
        coll,
        &json!({"data": {"_id": "dup1", "v": 2}}),
    );
    assert_eq!(status, 409, "{body}");
    assert_eq!(err_code(&body), "CONFLICT");
    assert_eq!(err_status(&body), 409);
}

#[test]
fn post_with_filter_updates() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "crud_post_update";
    clear_coll(&agent, &jwt, DB_CRUD, coll);
    seed(&agent, &jwt, DB_CRUD, coll, "n1", json!({"n": 1, "v": 0}));
    seed(&agent, &jwt, DB_CRUD, coll, "n2", json!({"n": 2, "v": 0}));

    let (status, body) = post_q(
        &agent,
        &jwt,
        DB_CRUD,
        coll,
        &json!({"filter": {"n": 1}, "data": {"v": 99}}),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["matched_count"], 1);
    assert_eq!(body["modified_count"], 1);

    let docs = get_docs(&agent, &jwt, DB_CRUD, coll);
    assert_eq!(docs.len(), 2);
    let by_id = |id: &str| {
        docs.iter()
            .find(|d| d["_id"].as_str() == Some(id))
            .expect("doc")
    };
    assert_eq!(by_id("n1")["v"], 99);
    assert_eq!(by_id("n2")["v"], 0);

    // no match -> 200 with zero counts (not an error)
    let (status, body) = post_q(
        &agent,
        &jwt,
        DB_CRUD,
        coll,
        &json!({"filter": {"n": 999}, "data": {"v": 1}}),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["matched_count"], 0);
    assert_eq!(body["modified_count"], 0);
}

#[test]
fn data_auto_wrapped_in_set() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "crud_set_wrap";
    clear_coll(&agent, &jwt, DB_CRUD, coll);
    seed(&agent, &jwt, DB_CRUD, coll, "s1", json!({"a": 1, "b": 2}));

    // clients send plain {field: value}; the server wraps it in $set
    let (status, body) = post_q(
        &agent,
        &jwt,
        DB_CRUD,
        coll,
        &json!({"filter": {"_id": "s1"}, "data": {"a": 9}}),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["matched_count"], 1);
    assert_eq!(body["modified_count"], 1);

    let (status, body) = get_filtered(&agent, &jwt, DB_CRUD, coll, &json!({"_id": "s1"}));
    assert_eq!(status, 200, "{body}");
    let doc = &body["documents"][0];
    assert_eq!(doc["a"], 9);
    assert_eq!(doc["b"], 2); // $set, not replace — untouched fields survive
}

#[test]
fn put_update_and_404() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "crud_put";
    clear_coll(&agent, &jwt, DB_CRUD, coll);
    seed(&agent, &jwt, DB_CRUD, coll, "p1", json!({"v": 1}));

    let (status, body) = put_q(
        &agent,
        &jwt,
        DB_CRUD,
        coll,
        &json!({"filter": {"_id": "p1"}, "data": {"v": 2}}),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["matched_count"], 1);
    assert_eq!(body["modified_count"], 1);

    // no match -> 404
    let (status, body) = put_q(
        &agent,
        &jwt,
        DB_CRUD,
        coll,
        &json!({"filter": {"_id": "missing"}, "data": {"v": 2}}),
    );
    assert_eq!(status, 404, "{body}");
    assert_eq!(err_code(&body), "NOT_FOUND");

    // missing filter -> 400
    let (status, body) = put_q(&agent, &jwt, DB_CRUD, coll, &json!({"data": {"v": 3}}));
    assert_eq!(status, 400, "{body}");
    assert_eq!(err_code(&body), "BAD_REQUEST");

    // empty filter -> 400 (would match everything)
    let (status, body) = put_q(
        &agent,
        &jwt,
        DB_CRUD,
        coll,
        &json!({"filter": {}, "data": {"v": 3}}),
    );
    assert_eq!(status, 400, "{body}");
    assert_eq!(err_code(&body), "BAD_REQUEST");
}

#[test]
fn patch_upsert_both_paths() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "crud_patch";
    clear_coll(&agent, &jwt, DB_CRUD, coll);

    // no match -> upsert (201)
    let (status, body) = patch_q(
        &agent,
        &jwt,
        DB_CRUD,
        coll,
        &json!({"filter": {"_id": "u1"}, "data": {"v": 1}}),
    );
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["matched_count"], 0);
    assert_eq!(body["modified_count"], 0);
    assert_eq!(body["upserted"], true);
    assert_eq!(body["upserted_id"], "u1");

    // match -> plain update (200)
    let (status, body) = patch_q(
        &agent,
        &jwt,
        DB_CRUD,
        coll,
        &json!({"filter": {"_id": "u1"}, "data": {"v": 2}}),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["matched_count"], 1);
    assert_eq!(body["modified_count"], 1);
    assert_eq!(body["upserted"], false);
    assert_eq!(body["upserted_id"], Value::Null);

    let (status, body) = get_filtered(&agent, &jwt, DB_CRUD, coll, &json!({"_id": "u1"}));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["documents"][0]["v"], 2);

    // missing filter -> 400
    let (status, body) = patch_q(&agent, &jwt, DB_CRUD, coll, &json!({"data": {"v": 3}}));
    assert_eq!(status, 400, "{body}");
    assert_eq!(err_code(&body), "BAD_REQUEST");
}

#[test]
fn delete_counts_and_404() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "crud_delete";
    clear_coll(&agent, &jwt, DB_CRUD, coll);
    for i in 0..3 {
        seed(
            &agent,
            &jwt,
            DB_CRUD,
            coll,
            &format!("d{i}"),
            json!({"group": "g", "i": i}),
        );
    }

    let (status, body) = delete_q(
        &agent,
        &jwt,
        DB_CRUD,
        coll,
        &json!({"filter": {"group": "g"}}),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["deleted_count"], 3);

    // nothing left -> 404
    let (status, body) = delete_q(
        &agent,
        &jwt,
        DB_CRUD,
        coll,
        &json!({"filter": {"group": "g"}}),
    );
    assert_eq!(status, 404, "{body}");
    assert_eq!(err_code(&body), "NOT_FOUND");

    // missing filter -> 400
    let (status, body) = delete_q(&agent, &jwt, DB_CRUD, coll, &json!({}));
    assert_eq!(status, 400, "{body}");
    assert_eq!(err_code(&body), "BAD_REQUEST");

    // $where filter -> 400 INVALID_FILTER (server-side scripts refused)
    let (status, body) = delete_q(
        &agent,
        &jwt,
        DB_CRUD,
        coll,
        &json!({"filter": {"$where": "true"}}),
    );
    assert_eq!(status, 400, "{body}");
    assert_eq!(err_code(&body), "INVALID_FILTER");
}

#[test]
fn invalid_write_bodies() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "crud_bad_body";

    // missing data -> 400
    let (status, body) = post_q(&agent, &jwt, DB_CRUD, coll, &json!({}));
    assert_eq!(status, 400, "{body}");
    assert_eq!(err_code(&body), "BAD_REQUEST");

    // data not an object -> 400
    let (status, body) = post_q(&agent, &jwt, DB_CRUD, coll, &json!({"data": [1, 2]}));
    assert_eq!(status, 400, "{body}");
    assert_eq!(err_code(&body), "BAD_REQUEST");
    let (status, body) = post_q(&agent, &jwt, DB_CRUD, coll, &json!({"data": "str"}));
    assert_eq!(status, 400, "{body}");
    assert_eq!(err_code(&body), "BAD_REQUEST");
    let (status, body) = post_q(&agent, &jwt, DB_CRUD, coll, &json!({"data": 7}));
    assert_eq!(status, 400, "{body}");
    assert_eq!(err_code(&body), "BAD_REQUEST");

    // filter not an object -> 400 INVALID_FILTER
    let (status, body) = post_q(
        &agent,
        &jwt,
        DB_CRUD,
        coll,
        &json!({"filter": [1], "data": {"x": 1}}),
    );
    assert_eq!(status, 400, "{body}");
    assert_eq!(err_code(&body), "INVALID_FILTER");
    let (status, body) = post_q(
        &agent,
        &jwt,
        DB_CRUD,
        coll,
        &json!({"filter": "x", "data": {"x": 1}}),
    );
    assert_eq!(status, 400, "{body}");
    assert_eq!(err_code(&body), "INVALID_FILTER");

    // body not JSON at all -> 400 with the standard error contract
    let url = format!("{}/q/{DB_CRUD}/{coll}", base());
    let res = agent
        .put(&url)
        .set("Authorization", &format!("Bearer {jwt}"))
        .set("Content-Type", "application/json")
        .send_string("this is not json");
    let (status, body) = finish(res);
    assert_eq!(status, 400, "{body}");
    assert_eq!(err_code(&body), "BAD_REQUEST");
    assert_eq!(err_status(&body), 400);
}

#[test]
fn unique_index_conflict_409() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "uniq";
    clear_coll(&agent, &jwt, DB_CRUD, coll);

    // a unique index on "email", created directly through the driver
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let client = mongodb::Client::with_uri_str(&mongo_uri()).await.unwrap();
        let coll = client
            .database("xdb_tb_crud")
            .collection::<bson::Document>("uniq");
        coll.create_index(
            mongodb::IndexModel::builder()
                .keys(bson::doc! {"email": 1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    });

    // unique email per run so index creation never sees stale duplicates
    let email = format!(
        "dupe-{}-{}@example.com",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    let (status, body) = post_q(
        &agent,
        &jwt,
        DB_CRUD,
        coll,
        &json!({"data": {"email": email, "v": 1}}),
    );
    assert_eq!(status, 201, "{body}");
    let (status, body) = post_q(
        &agent,
        &jwt,
        DB_CRUD,
        coll,
        &json!({"data": {"email": email, "v": 2}}),
    );
    assert_eq!(status, 409, "{body}");
    assert_eq!(err_code(&body), "CONFLICT");
}

#[test]
fn permission_gate_on_writes() {
    let agent = agent();
    let reader = jwt("reader");
    let (status, body) = post_q(
        &agent,
        &reader,
        DB_SHARED,
        "ro_gate",
        &json!({"data": {"_id": "x", "v": 1}}),
    );
    assert_eq!(status, 403, "{body}");
    assert_eq!(err_code(&body), "FORBIDDEN");
}
