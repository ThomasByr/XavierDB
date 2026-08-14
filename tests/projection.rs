//! GET /q projection semantics — include/exclude styles, _id handling,
//! invalid specs, and the union+strip interaction with sort fields during
//! keyset pagination. Every test seeds its own collection (db xdb_tb_proj).

mod common;

use common::*;
use serde_json::{Value, json};

const DB: &str = "xdb_tb_proj";

/// A doc with a nested object and an array, plus plain + secret fields.
fn seed_doc(agent: &ureq::Agent, jwt: &str, coll: &str, id: &str, name: &str, price: i64) {
    seed(
        agent,
        jwt,
        DB,
        coll,
        id,
        json!({
            "name": name,
            "price": price,
            "secret": "shh",
            "nested": {"a": 1, "b": "x"},
            "tags": ["t1", "t2"],
        }),
    );
}

fn doc_keys(doc: &Value) -> Vec<String> {
    let mut keys: Vec<String> = doc.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

#[test]
fn include_only() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "pj_include_only";
    seed_doc(&agent, &jwt, coll, "p1", "bob", 3);

    // union+strip: _id is stripped from the output unless _id:1 is explicit
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("projection", r#"{"name":1}"#)]);
    assert_eq!(s, 200, "{b}");
    let doc = &b["documents"][0];
    assert_eq!(doc_keys(doc), vec!["name"]);
    assert_eq!(doc["name"], "bob");

    // explicit _id:1 brings _id back
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("projection", r#"{"name":1,"_id":1}"#)],
    );
    assert_eq!(s, 200, "{b}");
    let doc = &b["documents"][0];
    assert_eq!(doc_keys(doc), vec!["_id", "name"]);
}

#[test]
fn exclude_only() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "pj_exclude_only";
    seed_doc(&agent, &jwt, coll, "p1", "bob", 3);

    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("projection", r#"{"secret":0}"#)]);
    assert_eq!(s, 200, "{b}");
    let doc = &b["documents"][0];
    assert_eq!(
        doc_keys(doc),
        vec!["_id", "name", "nested", "price", "tags"]
    );
    assert!(!doc.as_object().unwrap().contains_key("secret"));
    assert_eq!(doc["_id"], "p1");
    assert_eq!(doc["nested"], json!({"a": 1, "b": "x"}));
    assert_eq!(doc["tags"], json!(["t1", "t2"]));
}

#[test]
fn mixed_rejected() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "pj_mixed_rejected";
    seed_doc(&agent, &jwt, coll, "p1", "bob", 3);

    // mixing include and exclude (outside _id) is refused
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("projection", r#"{"a":1,"b":0}"#)],
    );
    assert_eq!(s, 400, "{b}");
    assert_eq!(err_code(&b), "INVALID_PROJECTION");

    // _id:0 is the one allowed exclusion next to includes
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("projection", r#"{"name":1,"_id":0}"#)],
    );
    assert_eq!(s, 200, "{b}");
    let doc = &b["documents"][0];
    assert_eq!(doc_keys(doc), vec!["name"]);
    assert_eq!(doc["name"], "bob");
}

#[test]
fn id_variants() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "pj_id_variants";
    seed_doc(&agent, &jwt, coll, "p1", "bob", 3);

    // _id:1 alone -> only _id
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("projection", r#"{"_id":1}"#)]);
    assert_eq!(s, 200, "{b}");
    let doc = &b["documents"][0];
    assert_eq!(doc_keys(doc), vec!["_id"]);
    assert_eq!(doc["_id"], "p1");

    // _id:0 alone: exclude-style projection -> every field except _id
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("projection", r#"{"_id":0}"#)]);
    assert_eq!(s, 200, "{b}");
    let doc = &b["documents"][0];
    let keys = doc_keys(doc);
    assert!(!keys.iter().any(|k| k == "_id"), "{keys:?}");
    assert!(
        keys.iter().any(|k| k == "name") && keys.iter().any(|k| k == "price"),
        "{keys:?}"
    );
}

#[test]
fn invalid_projections() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "pj_invalid_projections";
    seed_doc(&agent, &jwt, coll, "p1", "bob", 3);

    // bad values
    for proj in [r#"{"name":2}"#, r#"{"name":"yes"}"#, r#"{"name":null}"#] {
        let (s, b) = get_q(&agent, &jwt, DB, coll, &[("projection", proj)]);
        assert_eq!(s, 400, "{proj} -> {b}");
        assert_eq!(err_code(&b), "INVALID_PROJECTION");
    }
    // dotted key
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("projection", r#"{"a.b":1}"#)]);
    assert_eq!(s, 400, "{b}");
    assert_eq!(err_code(&b), "INVALID_PROJECTION");
    // $-prefixed operator key
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("projection", r#"{"$foo":1}"#)]);
    assert_eq!(s, 400, "{b}");
    assert_eq!(err_code(&b), "INVALID_PROJECTION");

    // {} is a no-op: full document, _id included
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("projection", r#"{}"#)]);
    assert_eq!(s, 200, "{b}");
    let doc = &b["documents"][0];
    assert_eq!(
        doc_keys(doc),
        vec!["_id", "name", "nested", "price", "secret", "tags"]
    );
}

#[test]
fn projection_with_sort_strips_sort_fields() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "pj_projection_with_sort";
    // prices intentionally out of name order
    for (id, name, price) in [
        ("p1", "n1", 10),
        ("p2", "n2", 3),
        ("p3", "n3", 25),
        ("p4", "n4", 1),
        ("p5", "n5", 20),
        ("p6", "n6", 5),
    ] {
        seed_doc(&agent, &jwt, coll, id, name, price);
    }

    // reference: unprojected price-asc order of names
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("sort", r#"{"price":1}"#)]);
    assert_eq!(s, 200, "{b}");
    let ref_names: Vec<String> = b["documents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ref_names, vec!["n4", "n2", "n6", "n1", "n5", "n3"]);

    // projected: same order, but every doc has ONLY name (price + _id are
    // force-added for the keyset machinery and stripped from the output)
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("sort", r#"{"price":1}"#), ("projection", r#"{"name":1}"#)],
    );
    assert_eq!(s, 200, "{b}");
    let docs = b["documents"].as_array().unwrap();
    let names: Vec<String> = docs
        .iter()
        .map(|d| d["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ref_names);
    for d in docs {
        assert_eq!(doc_keys(d), vec!["name"]);
    }
}

#[test]
fn projection_pagination_stable() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "pj_projection_pagination_stable";
    for i in 0..10 {
        let name = format!("n{i}");
        let price = 10 + i; // distinct prices in _id order
        seed_doc(&agent, &jwt, coll, &format!("p{i}"), &name, price);
    }

    let mut cursor: Option<String> = None;
    let mut seen: Vec<String> = Vec::new();
    loop {
        let mut params = vec![("sort", r#"{"price":1}"#), ("projection", r#"{"name":1}"#)];
        if let Some(c) = &cursor {
            params.push(("cursor", c));
        }
        let (s, b) = get_q(&agent, &jwt, DB, coll, &params);
        assert_eq!(s, 200, "{b}");
        let docs = b["documents"].as_array().unwrap();
        for d in docs {
            // price and _id stay stripped on every page
            assert_eq!(doc_keys(d), vec!["name"]);
            seen.push(d["name"].as_str().unwrap().to_string());
        }
        if b["has_more"] == false {
            break;
        }
        cursor = b["next_cursor"].as_str().map(|s| s.to_string());
    }

    // every doc exactly once, in price order
    assert_eq!(
        seen,
        vec!["n0", "n1", "n2", "n3", "n4", "n5", "n6", "n7", "n8", "n9"]
    );
    let mut sorted = seen.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 10);
}
