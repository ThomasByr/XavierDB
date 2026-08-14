//! Multi-app real-world scenarios — several distinct apps (full-access,
//! restricted, read-only, ingester, collection-scoped) interacting with
//! shared and private datasets through the /q proxy, plus live block/perms
//! mutation, the adaptive limit cap and read-your-writes semantics.
//! All requests use cached JWTs (no fresh /auth).

mod common;

use common::*;
use serde_json::{Value, json};
use std::collections::HashSet;

fn uniq(prefix: &str) -> String {
    format!("{prefix}_{}", std::process::id())
}

fn doc_ids(body: &Value) -> Vec<String> {
    body["documents"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|d| d["_id"].as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn shared_collection_across_apps() {
    ensure_server();
    let agent = agent();
    let main = jwt("main");
    let coll = uniq("ma_shared");
    seed(
        &agent,
        &main,
        DB_SHARED,
        &coll,
        "ma-s-1",
        json!({ "v": 10, "tag": "a" }),
    );
    seed(
        &agent,
        &main,
        DB_SHARED,
        &coll,
        "ma-s-2",
        json!({ "v": 20, "tag": "b" }),
    );
    seed(
        &agent,
        &main,
        DB_SHARED,
        &coll,
        "ma-s-3",
        json!({ "v": 30, "tag": "c" }),
    );

    // reader: GET-only on xdb_tb_shared — sees the same documents
    let (s, b) = get_q(&agent, &jwt("reader"), DB_SHARED, &coll, &[]);
    assert_eq!(s, 200, "{b}");
    let mut ids = doc_ids(&b);
    ids.sort();
    assert_eq!(ids, vec!["ma-s-1", "ma-s-2", "ma-s-3"]);
    let vs: Vec<i64> = b["documents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["v"].as_i64().unwrap())
        .collect();
    assert!(vs.contains(&10) && vs.contains(&20) && vs.contains(&30));

    // ruser: GET on * (except the secret db) — same documents visible
    let (s, b) = get_q(&agent, &jwt("ruser"), DB_SHARED, &coll, &[]);
    assert_eq!(s, 200, "{b}");
    let mut ids = doc_ids(&b);
    ids.sort();
    assert_eq!(ids, vec!["ma-s-1", "ma-s-2", "ma-s-3"]);
}

#[test]
fn private_db_isolation() {
    ensure_server();
    let agent = agent();
    let main = jwt("main");
    let coll = uniq("ma_secret");
    seed(
        &agent,
        &main,
        DB_SECRET,
        &coll,
        "ma-secret-1",
        json!({ "v": 1, "owner": "tester" }),
    );

    // reader (ro app: GET only xdb_tb_shared) — the secret db stays invisible
    let (s, b) = get_q(&agent, &jwt("reader"), DB_SECRET, &coll, &[]);
    assert_eq!(s, 403, "{b}");
    assert_eq!(b["code"], "FORBIDDEN");

    // ruser has a global GET * but the app-level deny beats the allow
    let (s, b) = get_q(&agent, &jwt("ruser"), DB_SECRET, &coll, &[]);
    assert_eq!(s, 403, "{b}");
    assert_eq!(b["code"], "FORBIDDEN");
}

#[test]
fn concurrent_writers() {
    ensure_server();
    let a = agent();
    let main = jwt("main");
    let coll = uniq("ma_conc");
    clear_coll(&a, &main, DB_SHARED, &coll);

    // two identities of the same full app write in parallel
    std::thread::scope(|s| {
        let c1 = coll.clone();
        let h1 = s.spawn(move || {
            let a = agent();
            let j = jwt("main");
            for i in 0..20 {
                let (st, b) = post_q(
                    &a,
                    &j,
                    DB_SHARED,
                    &c1,
                    &json!({ "data": { "_id": format!("t1-{i}"), "v": i } }),
                );
                assert_eq!(st, 201, "t1-{i}: {b}");
            }
        });
        let c2 = coll.clone();
        let h2 = s.spawn(move || {
            let a = agent();
            let j = jwt("main2");
            for i in 0..20 {
                let (st, b) = post_q(
                    &a,
                    &j,
                    DB_SHARED,
                    &c2,
                    &json!({ "data": { "_id": format!("t2-{i}"), "v": i } }),
                );
                assert_eq!(st, 201, "t2-{i}: {b}");
            }
        });
        h1.join().unwrap();
        h2.join().unwrap();
    });

    // both writers' docs landed, none lost
    assert_eq!(count_docs(&a, &main, DB_SHARED, &coll), 40);
    let (s, b) = get_q(&a, &main, DB_SHARED, &coll, &[]);
    assert_eq!(s, 200, "{b}");
    let mut seen: HashSet<String> = doc_ids(&b).into_iter().collect();
    assert_eq!(seen.len(), 40, "{b}");
    for i in 0..20 {
        assert!(seen.remove(&format!("t1-{i}")), "missing t1-{i}");
        assert!(seen.remove(&format!("t2-{i}")), "missing t2-{i}");
    }
    assert!(seen.is_empty());
}

#[test]
fn ingester_pipeline() {
    ensure_server();
    // u2 belongs to xdb_tb_m2, which app_deleted_perms_change_live temporarily
    // removes from perms — serialize both tests under the suite lock.
    let _g = suite_lock().lock().unwrap();
    let agent = agent();
    let main = jwt("main");
    let coll = uniq("ma_pipe");
    clear_coll(&agent, &main, DB_SHARED, &coll);

    // u2 (POST+PATCH only) feeds the collection
    let u2 = jwt("u2");
    for i in 1..=3 {
        let (st, b) = post_q(
            &agent,
            &u2,
            DB_SHARED,
            &coll,
            &json!({ "data": { "_id": format!("ma-ing-{i}"), "v": i, "source": "u2" } }),
        );
        assert_eq!(st, 201, "insert {i}: {b}");
    }

    // asymmetric visibility: the ingester cannot read back its own writes
    let (s, b) = get_q(&agent, &u2, DB_SHARED, &coll, &[]);
    assert_eq!(s, 403, "{b}");
    assert_eq!(b["code"], "FORBIDDEN");

    // readers see the ingested docs
    let (s, b) = get_q(&agent, &jwt("reader"), DB_SHARED, &coll, &[]);
    assert_eq!(s, 200, "{b}");
    let mut ids = doc_ids(&b);
    ids.sort();
    assert_eq!(ids, vec!["ma-ing-1", "ma-ing-2", "ma-ing-3"]);
    for d in b["documents"].as_array().unwrap() {
        assert_eq!(d["source"], "u2");
    }
    assert_eq!(count_docs(&agent, &main, DB_SHARED, &coll), 3);
}

#[test]
fn collection_scoped_app() {
    ensure_server();
    let agent = agent();
    let main = jwt("main");
    // the fixture scopes u3 to the literal collection "public" — fixed ids
    seed(
        &agent,
        &main,
        DB_SHARED,
        "public",
        "ma-pub-1",
        json!({ "v": 1 }),
    );
    seed(
        &agent,
        &main,
        DB_SHARED,
        "public",
        "ma-pub-2",
        json!({ "v": 2 }),
    );

    let u3 = jwt("u3");
    let (s, b) = get_q(&agent, &u3, DB_SHARED, "public", &[]);
    assert_eq!(s, 200, "{b}");
    let ids = doc_ids(&b);
    assert!(ids.contains(&"ma-pub-1".to_string()), "{b}");
    assert!(ids.contains(&"ma-pub-2".to_string()), "{b}");

    // any other collection in the same db is off-limits
    let (s, b) = get_q(&agent, &u3, DB_SHARED, "ma_private", &[]);
    assert_eq!(s, 403, "{b}");
    assert_eq!(b["code"], "FORBIDDEN");
}

#[test]
fn name_block_isolation() {
    ensure_server();
    let _g = suite_lock().lock().unwrap();
    let agent = agent();
    let cookie = dash_cookie();

    // block only reader2@xdb_tb_ro — reader keeps full access
    let (s, b) = dash_post(
        &agent,
        &cookie,
        "/dashboard/api/block",
        Some(&json!({ "id": "reader2@xdb_tb_ro" })),
    );
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["ok"], true);

    let (s, b) = get_q(&agent, &jwt("reader2"), DB_SHARED, "seed", &[]);
    assert_eq!(s, 403, "{b}");
    assert_eq!(b["code"], "BLOCKED");
    let (s, b) = get_q(&agent, &jwt("reader"), DB_SHARED, "seed", &[]);
    assert_eq!(s, 200, "{b}");

    let (s, b) = dash_post(
        &agent,
        &cookie,
        "/dashboard/api/unblock",
        Some(&json!({ "id": "reader2@xdb_tb_ro" })),
    );
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["ok"], true);
    let (s, b) = get_q(&agent, &jwt("reader2"), DB_SHARED, "seed", &[]);
    assert_eq!(s, 200, "{b}");
}

#[test]
fn app_deleted_perms_change_live() {
    ensure_server();
    let _g = suite_lock().lock().unwrap();
    let agent = agent();
    let cookie = dash_cookie();

    let m2 = json!({
        "app": "xdb_tb_m2",
        "set_token": "tb-m2-secret-token",
        "allow": [{ "actions": ["POST", "PATCH"], "databases": ["xdb_tb_shared"] }],
        "deny": [],
    });

    // defensive re-add: a failed earlier run may have left the app deleted
    let (s, b) = dash_get(&agent, &cookie, "/dashboard/api/perms");
    assert_eq!(s, 200, "{b}");
    let exists = b["apps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["app"] == "xdb_tb_m2");
    if !exists {
        let (s, b) = dash_post(
            &agent,
            &cookie,
            "/dashboard/api/perms",
            Some(&json!({ "apps": [m2.clone()] })),
        );
        assert_eq!(s, 200, "{b}");
        assert_eq!(b["ok"], true);
    }

    let u2 = jwt("u2");
    let coll = uniq("ma_del");

    // delete the whole app — live perms lose the rule; the JWT stays valid
    let (s, b) = dash_post(
        &agent,
        &cookie,
        "/dashboard/api/perms",
        Some(&json!({ "apps": [{ "app": "xdb_tb_m2", "delete": true }] })),
    );
    assert_eq!(s, 200, "{b}");
    let (s, b) = post_q(
        &agent,
        &u2,
        DB_SHARED,
        &coll,
        &json!({ "data": { "_id": "ma-del-1", "v": 1 } }),
    );
    assert_eq!(s, 403, "{b}");
    assert_eq!(b["code"], "FORBIDDEN");

    // re-create with the same token and rules; the cached u2 JWT works again
    let (s, b) = dash_post(
        &agent,
        &cookie,
        "/dashboard/api/perms",
        Some(&json!({ "apps": [m2] })),
    );
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["ok"], true);
    let (s, b) = post_q(
        &agent,
        &u2,
        DB_SHARED,
        &coll,
        &json!({ "data": { "_id": "ma-del-1", "v": 1 } }),
    );
    assert_eq!(s, 201, "{b}");
    assert_eq!(b["inserted_count"], 1);
}

#[test]
fn adaptive_limit_cap() {
    ensure_server();
    let agent = agent();
    let main = jwt("main");
    let coll = uniq("ma_limit");
    clear_coll(&agent, &main, DB_SHARED, &coll);
    for i in 0..250 {
        seed(
            &agent,
            &main,
            DB_SHARED,
            &coll,
            &format!("d-{i}"),
            json!({ "v": i }),
        );
    }

    // a huge requested limit is capped at the enforced adaptive limit
    let (s, b) = get_q(&agent, &main, DB_SHARED, &coll, &[("limit", "10000")]);
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["truncated"], true);
    assert_eq!(b["has_more"], true);
    assert_eq!(b["count"].as_u64().unwrap(), 200);
    assert!(b["limit_applied"].as_u64().unwrap() <= 200);

    // walking the cursor still reaches every document
    let mut seen: HashSet<String> = doc_ids(&b).into_iter().collect();
    let mut cursor = b["next_cursor"].as_str().map(|c| c.to_string());
    let mut pages = 1;
    while let Some(c) = cursor {
        let (s, b) = get_q(
            &agent,
            &main,
            DB_SHARED,
            &coll,
            &[("limit", "10000"), ("cursor", &c)],
        );
        assert_eq!(s, 200, "{b}");
        for d in b["documents"].as_array().unwrap() {
            seen.insert(d["_id"].as_str().unwrap().to_string());
        }
        pages += 1;
        assert!(pages <= 10, "runaway pagination");
        cursor = match b["has_more"].as_bool() {
            Some(true) => b["next_cursor"].as_str().map(|c| c.to_string()),
            _ => None,
        };
    }
    assert_eq!(seen.len(), 250);
    for i in 0..250 {
        assert!(seen.contains(&format!("d-{i}")), "missing d-{i}");
    }
}

#[test]
fn read_your_writes() {
    ensure_server();
    let w = agent();
    let main = jwt("main");
    let coll = uniq("ma_ryw");

    let (s, b) = post_q(
        &w,
        &main,
        DB_SHARED,
        &coll,
        &json!({ "data": { "_id": "ma-ryw-1", "name": "alice", "v": 1 } }),
    );
    assert_eq!(s, 201, "{b}");
    assert_eq!(b["inserted_count"], 1);
    assert_eq!(b["inserted_id"], "ma-ryw-1");

    // immediate read-back (fresh agent, same app) sees the full document
    let reader = agent();
    let filter = json!({ "_id": "ma-ryw-1" }).to_string();
    let (s, b) = get_q(&reader, &main, DB_SHARED, &coll, &[("filter", &filter)]);
    assert_eq!(s, 200, "{b}");
    let docs = b["documents"].as_array().unwrap();
    assert_eq!(docs.len(), 1, "{b}");
    assert_eq!(docs[0]["_id"], "ma-ryw-1");
    assert_eq!(docs[0]["name"], "alice");
    assert_eq!(docs[0]["v"], 1);

    // upsert an extra field onto the same doc -> updated, not duplicated
    let (s, b) = patch_q(
        &w,
        &main,
        DB_SHARED,
        &coll,
        &json!({ "filter": { "_id": "ma-ryw-1" }, "data": { "extra": true } }),
    );
    assert_eq!(s, 200, "{b}");

    let (s, b) = get_q(&reader, &main, DB_SHARED, &coll, &[("filter", &filter)]);
    assert_eq!(s, 200, "{b}");
    let docs = b["documents"].as_array().unwrap();
    assert_eq!(docs.len(), 1, "{b}");
    assert_eq!(docs[0]["extra"], true);
    assert_eq!(docs[0]["name"], "alice");
}
