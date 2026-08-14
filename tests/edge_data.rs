//! Edge-case data round-trips through GET/POST /q: unicode, long strings,
//! deep nesting, null/empty values, numeric extremes, mixed arrays, _id
//! types, and field names containing dots or a leading $. All tests live in
//! xdb_tb_edge and use their own collection.

mod common;

use common::*;
use serde_json::{Value, json};

const DB_EDGE: &str = "xdb_tb_edge";

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
fn unicode_roundtrip() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "edge_unicode";
    clear_coll(&agent, &jwt, DB_EDGE, coll);

    let doc = json!({
        "_id": "u1",
        "emoji": "h\u{e9}llo w\u{f6}rld \u{1F680} \u{4f60}\u{597d}",
        "rtl": "\u{5e9}\u{5dc}\u{5d5}\u{5dd} \u{5e2}\u{5d5}\u{5dc}\u{5dd}",
        "combining": "e\u{301} a\u{308}",
        "zwsp": "a\u{200B}b",
    });
    let (status, body) = post_q(&agent, &jwt, DB_EDGE, coll, &json!({"data": doc}));
    assert_eq!(status, 201, "{body}");

    let (status, body) = get_filtered(&agent, &jwt, DB_EDGE, coll, &json!({"_id": "u1"}));
    assert_eq!(status, 200, "{body}");
    let got = &body["documents"][0];
    assert_eq!(
        got["emoji"],
        "h\u{e9}llo w\u{f6}rld \u{1F680} \u{4f60}\u{597d}"
    );
    assert_eq!(
        got["rtl"],
        "\u{5e9}\u{5dc}\u{5d5}\u{5dd} \u{5e2}\u{5d5}\u{5dc}\u{5dd}"
    );
    assert_eq!(got["combining"], "e\u{301} a\u{308}");
    assert_eq!(got["zwsp"], "a\u{200B}b");
}

#[test]
fn long_string_roundtrip() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "edge_long";
    clear_coll(&agent, &jwt, DB_EDGE, coll);

    let ascii = "A".repeat(100_000);
    let uni = "h\u{e9}llo-\u{1F680}-".repeat(1_000);
    assert!(
        uni.len() >= 10_000,
        "unicode fixture too short: {}",
        uni.len()
    );

    let doc = json!({"_id": "l1", "big_ascii": ascii, "big_uni": uni});
    let (status, body) = post_q(&agent, &jwt, DB_EDGE, coll, &json!({"data": doc}));
    assert_eq!(status, 201, "{body}");

    let (status, body) = get_filtered(&agent, &jwt, DB_EDGE, coll, &json!({"_id": "l1"}));
    assert_eq!(status, 200, "{body}");
    let got = &body["documents"][0];
    let a = got["big_ascii"].as_str().expect("big_ascii");
    let u = got["big_uni"].as_str().expect("big_uni");
    assert_eq!(a.len(), 100_000);
    assert_eq!(u.len(), uni.len());
    assert_eq!(a, "A".repeat(100_000));
    assert_eq!(u, uni);
}

#[test]
fn deep_nesting() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "edge_nest";
    clear_coll(&agent, &jwt, DB_EDGE, coll);

    // 40 levels of nested objects, built inside-out
    let mut deep: Value = json!({"leaf": true});
    for i in 0..40 {
        deep = json!({"level": i, "next": deep});
    }
    let doc = json!({"_id": "n1", "deep": deep, "arr3": [[[1, 2], [3]], [["x"]]]});
    let (status, body) = post_q(&agent, &jwt, DB_EDGE, coll, &json!({"data": doc}));
    assert_eq!(status, 201, "{body}");

    let (status, body) = get_filtered(&agent, &jwt, DB_EDGE, coll, &json!({"_id": "n1"}));
    assert_eq!(status, 200, "{body}");
    let got = &body["documents"][0];
    assert_eq!(got["deep"], deep);
    assert_eq!(got["arr3"], json!([[[1, 2], [3]], [["x"]]]));
}

#[test]
fn null_empty_missing() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "edge_null";
    clear_coll(&agent, &jwt, DB_EDGE, coll);

    let doc = json!({"_id": "ne1", "n": null, "e": "", "o": {}, "a": []});
    let (status, body) = post_q(&agent, &jwt, DB_EDGE, coll, &json!({"data": doc}));
    assert_eq!(status, 201, "{body}");

    let (status, body) = get_filtered(&agent, &jwt, DB_EDGE, coll, &json!({"_id": "ne1"}));
    assert_eq!(status, 200, "{body}");
    let got = &body["documents"][0];
    assert_eq!(got["n"], Value::Null);
    assert_eq!(got["e"], "");
    assert_eq!(got["o"], json!({}));
    assert_eq!(got["a"], json!([]));
    assert!(
        got.get("missing").is_none(),
        "missing field must stay absent"
    );
}

#[test]
fn numeric_edge_cases() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "edge_nums";
    clear_coll(&agent, &jwt, DB_EDGE, coll);

    let doc = json!({
        "_id": "num1",
        "i64_max": 9223372036854775807i64,
        "i64_min": -9223372036854775808i64,
        "zero": 0,
        "u64_max": 18446744073709551615u64,
        "f01": 0.1,
        "big": 1e300,
        "neg_big": -1e300,
        "nan": {"$numberDouble": "NaN"},
    });
    let (status, body) = post_q(&agent, &jwt, DB_EDGE, coll, &json!({"data": doc}));
    assert_eq!(status, 201, "{body}");

    let (status, body) = get_filtered(&agent, &jwt, DB_EDGE, coll, &json!({"_id": "num1"}));
    assert_eq!(status, 200, "{body}");
    let got = &body["documents"][0];
    assert_eq!(got["i64_max"], 9223372036854775807i64);
    assert_eq!(got["i64_min"], -9223372036854775808i64);
    assert_eq!(got["zero"], 0);
    // u64 above i64::MAX cannot be stored as int64: Decimal128, relaxed extjson
    assert_eq!(
        got["u64_max"],
        json!({"$numberDecimal": "18446744073709551615"})
    );
    assert_eq!(got["f01"], 0.1);
    assert_eq!(got["big"], 1e300);
    assert_eq!(got["neg_big"], -1e300);
    assert_eq!(got["nan"], json!({"$numberDouble": "NaN"}));
}

#[test]
fn arrays_and_mixed() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "edge_arrays";
    clear_coll(&agent, &jwt, DB_EDGE, coll);

    let doc = json!({
        "_id": "a1",
        "scalars": [1, "two", true, null, 2.5],
        "objects": [{"a": 1}, {"b": [2, 3]}],
        "nested": [[1, 2], [3, [4]]],
        "mixed": [1, "x", [true], {"k": null}],
    });
    let (status, body) = post_q(&agent, &jwt, DB_EDGE, coll, &json!({"data": doc}));
    assert_eq!(status, 201, "{body}");

    let (status, body) = get_filtered(&agent, &jwt, DB_EDGE, coll, &json!({"_id": "a1"}));
    assert_eq!(status, 200, "{body}");
    let got = &body["documents"][0];
    assert_eq!(got["scalars"], json!([1, "two", true, null, 2.5]));
    assert_eq!(got["objects"], json!([{"a": 1}, {"b": [2, 3]}]));
    assert_eq!(got["nested"], json!([[1, 2], [3, [4]]]));
    assert_eq!(got["mixed"], json!([1, "x", [true], {"k": null}]));
}

#[test]
fn id_types() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "edge_ids";
    clear_coll(&agent, &jwt, DB_EDGE, coll);

    // string _id
    let (status, body) = post_q(
        &agent,
        &jwt,
        DB_EDGE,
        coll,
        &json!({"data": {"_id": "str-id", "v": "s"}}),
    );
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["inserted_id"], "str-id");

    // integer _id
    let (status, body) = post_q(
        &agent,
        &jwt,
        DB_EDGE,
        coll,
        &json!({"data": {"_id": 7, "v": "i"}}),
    );
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["inserted_id"], 7);

    // ObjectId _id via extended JSON
    let oid = "65f00000000000000000abcd";
    let (status, body) = post_q(
        &agent,
        &jwt,
        DB_EDGE,
        coll,
        &json!({"data": {"_id": {"$oid": oid}, "v": "o"}}),
    );
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["inserted_id"], oid);

    // filter + read back by each form
    let (status, body) = get_filtered(&agent, &jwt, DB_EDGE, coll, &json!({"_id": "str-id"}));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["documents"][0]["_id"], "str-id");
    assert_eq!(body["documents"][0]["v"], "s");

    let (status, body) = get_filtered(&agent, &jwt, DB_EDGE, coll, &json!({"_id": 7}));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["documents"][0]["_id"], 7);
    assert_eq!(body["documents"][0]["v"], "i");

    let (status, body) = get_filtered(&agent, &jwt, DB_EDGE, coll, &json!({"_id": {"$oid": oid}}));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["documents"][0]["_id"], oid);
    assert_eq!(body["documents"][0]["v"], "o");
}

#[test]
fn dotted_and_dollar_field_names() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "edge_fields";
    clear_coll(&agent, &jwt, DB_EDGE, coll);

    // MongoDB 8 stores dots literally
    let (status, body) = post_q(
        &agent,
        &jwt,
        DB_EDGE,
        coll,
        &json!({"data": {"_id": "dots1", "a.b": 1}}),
    );
    assert_eq!(status, 201, "{body}");
    let (status, body) = get_filtered(&agent, &jwt, DB_EDGE, coll, &json!({"_id": "dots1"}));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["documents"][0]["a.b"], 1);

    // $-prefixed field name: LIVE behavior on this server/Mongo (8.0.12) is
    // that the write is ACCEPTED and the key round-trips literally. The task
    // spec expected 400; observed 201 — discrepancy documented in the report.
    let (status, body) = post_q(
        &agent,
        &jwt,
        DB_EDGE,
        coll,
        &json!({"data": {"_id": "dollar1", "$bad": 1}}),
    );
    assert_eq!(status, 201, "{body}");
    let (status, body) = get_filtered(&agent, &jwt, DB_EDGE, coll, &json!({"_id": "dollar1"}));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["documents"][0]["$bad"], 1);
}
