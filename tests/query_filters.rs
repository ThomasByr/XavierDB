//! GET /q filter/sort/limit semantics — comparison operators, extended JSON,
//! regex, $exists/$type, dates, ObjectIds, sort variants, limit behavior and
//! invalid inputs. Every test seeds its own collection (db xdb_tb_query).

mod common;

use common::*;
use serde_json::{Value, json};

const DB: &str = "xdb_tb_query";

/// The standard 6-doc dataset. Prices mix ints (10/25/3/20), NaN and
/// Decimal128 (1.25); `opt` is present on q1/q3/q5 only; names include one
/// capitalized "Apple" (q4) for case-sensitivity checks.
fn seed_set(agent: &ureq::Agent, jwt: &str, coll: &str) {
    seed(
        agent,
        jwt,
        DB,
        coll,
        "q1",
        json!({
            "name": "apple", "price": 10, "tag": "fruit",
            "harvested": {"$date": "2026-07-01T00:00:00Z"}, "opt": "x",
        }),
    );
    seed(
        agent,
        jwt,
        DB,
        coll,
        "q2",
        json!({
            "name": "banana", "price": 25, "tag": "fruit",
            "harvested": {"$date": "2026-06-15T00:00:00Z"},
        }),
    );
    seed(
        agent,
        jwt,
        DB,
        coll,
        "q3",
        json!({
            "name": "carrot", "price": 3, "tag": "veggie",
            "harvested": {"$date": "2026-08-01T00:00:00Z"}, "opt": "y",
        }),
    );
    seed(
        agent,
        jwt,
        DB,
        coll,
        "q4",
        json!({
            "name": "Apple", "price": 20, "tag": "fruit",
            "harvested": {"$date": "2026-07-20T00:00:00Z"},
        }),
    );
    seed(
        agent,
        jwt,
        DB,
        coll,
        "q5",
        json!({
            "name": "durian", "price": {"$numberDouble": "NaN"}, "tag": "exotic",
            "harvested": {"$date": "2026-05-01T00:00:00Z"}, "opt": "z",
        }),
    );
    seed(
        agent,
        jwt,
        DB,
        coll,
        "q6",
        json!({
            "name": "elder", "price": {"$numberDecimal": "1.25"}, "tag": "exotic",
            "harvested": {"$date": "2026-07-05T00:00:00Z"},
        }),
    );
}

fn doc_ids(docs: &[Value]) -> Vec<String> {
    docs.iter()
        .map(|d| d["_id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn equality_and_in_ne() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "qf_equality_and_in_ne";
    seed_set(&agent, &jwt, coll);

    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("filter", r#"{"name":"apple"}"#)]);
    assert_eq!(s, 200, "{b}");
    let docs = b["documents"].as_array().unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["_id"], "q1");

    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("filter", r#"{"name":{"$in":["apple","banana"]}}"#)],
    );
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["documents"].as_array().unwrap().len(), 2);

    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("filter", r#"{"name":{"$ne":"apple"}}"#)],
    );
    assert_eq!(s, 200, "{b}");
    let docs = b["documents"].as_array().unwrap();
    assert_eq!(docs.len(), 5);
    // "Apple" (q4) is a different string and stays in the $ne result set
    assert!(docs.iter().all(|d| d["name"] != "apple"));
    assert!(docs.iter().any(|d| d["name"] == "Apple"));
}

#[test]
fn numeric_ranges() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "qf_numeric_ranges";
    seed_set(&agent, &jwt, coll);

    // ints only: NaN never matches a comparison; Decimal128 1.25 < 10
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("filter", r#"{"price":{"$gte":10,"$lt":26}}"#)],
    );
    assert_eq!(s, 200, "{b}");
    let docs = b["documents"].as_array().unwrap();
    assert_eq!(doc_ids(docs), vec!["q1", "q2", "q4"]);

    // every numeric doc (incl. the NaN one? no: NaN comparisons never match)
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("filter", r#"{"price":{"$gte":0}}"#)],
    );
    assert_eq!(s, 200, "{b}");
    let docs = b["documents"].as_array().unwrap();
    assert_eq!(docs.len(), 5);
    assert!(docs.iter().all(|d| d["_id"] != "q5"));

    // Decimal128 boundary: Mongo compares numerics across types by value, so
    // every numeric price >= 1.00 matches — not just the decimal doc
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("filter", r#"{"price":{"$gte":{"$numberDecimal":"1.00"}}}"#)],
    );
    assert_eq!(s, 200, "{b}");
    let docs = b["documents"].as_array().unwrap();
    assert_eq!(docs.len(), 5);
    assert!(docs.iter().all(|d| d["_id"] != "q5"));

    // extended-JSON round-trips: NaN and Decimal128 values come back wrapped
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("filter", r#"{"_id":"q5"}"#)]);
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["documents"][0]["price"], json!({"$numberDouble": "NaN"}));
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("filter", r#"{"_id":"q6"}"#)]);
    assert_eq!(s, 200, "{b}");
    assert_eq!(
        b["documents"][0]["price"],
        json!({"$numberDecimal": "1.25"})
    );
}

#[test]
fn regex_and_options() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "qf_regex_and_options";
    seed_set(&agent, &jwt, coll);

    // case-insensitive ^a matches "apple" and "Apple"
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("filter", r#"{"name":{"$regex":"^a","$options":"i"}}"#)],
    );
    assert_eq!(s, 200, "{b}");
    let docs = b["documents"].as_array().unwrap();
    assert_eq!(doc_ids(docs), vec!["q1", "q4"]);

    // without $options the match is case-sensitive
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("filter", r#"{"name":{"$regex":"^a"}}"#)],
    );
    assert_eq!(s, 200, "{b}");
    let docs = b["documents"].as_array().unwrap();
    assert_eq!(doc_ids(docs), vec!["q1"]);

    // bad regex is a client-caused Mongo error -> 400 BAD_REQUEST
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("filter", r#"{"name":{"$regex":"["}}"#)],
    );
    assert_eq!(s, 400, "{b}");
    assert_eq!(err_code(&b), "BAD_REQUEST");
}

#[test]
fn exists_and_type() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "qf_exists_and_type";
    seed_set(&agent, &jwt, coll);

    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("filter", r#"{"opt":{"$exists":true}}"#)],
    );
    assert_eq!(s, 200, "{b}");
    let docs = b["documents"].as_array().unwrap();
    assert_eq!(doc_ids(docs), vec!["q1", "q3", "q5"]);

    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("filter", r#"{"opt":{"$exists":false}}"#)],
    );
    assert_eq!(s, 200, "{b}");
    assert_eq!(
        doc_ids(b["documents"].as_array().unwrap()),
        vec!["q2", "q4", "q6"]
    );

    // string field
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("filter", r#"{"name":{"$type":"string"}}"#)],
    );
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["documents"].as_array().unwrap().len(), 6);

    // price is mixed int/double(NaN)/decimal -> all are BSON numbers
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("filter", r#"{"price":{"$type":"number"}}"#)],
    );
    assert_eq!(s, 200, "{b}");
    let docs = b["documents"].as_array().unwrap();
    // NaN is a BSON double, so the NaN doc counts as a number too
    assert_eq!(docs.len(), 6);
    assert!(docs.iter().any(|d| d["_id"] == "q5"));

    // opt is string-or-missing: only the string docs match
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("filter", r#"{"opt":{"$type":"string"}}"#)],
    );
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["documents"].as_array().unwrap().len(), 3);
}

#[test]
fn date_filters() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "qf_date_filters";
    seed_set(&agent, &jwt, coll);

    // harvested >= 2026-07-01: q1 (exactly at the boundary), q6, q4
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[(
            "filter",
            r#"{"harvested":{"$gte":{"$date":"2026-07-01T00:00:00Z"}}}"#,
        )],
    );
    assert_eq!(s, 200, "{b}");
    assert_eq!(
        doc_ids(b["documents"].as_array().unwrap()),
        vec!["q1", "q3", "q4", "q6"]
    );

    // harvested < 2026-07-01: q2 (06-15) and q5 (05-01); q1 is excluded
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[(
            "filter",
            r#"{"harvested":{"$lt":{"$date":"2026-07-01T00:00:00Z"}}}"#,
        )],
    );
    assert_eq!(s, 200, "{b}");
    assert_eq!(
        doc_ids(b["documents"].as_array().unwrap()),
        vec!["q2", "q5"]
    );

    // dates round-trip as chrono RFC3339 (+00:00, not Z)
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("filter", r#"{"_id":"q1"}"#)]);
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["documents"][0]["harvested"], "2026-07-01T00:00:00+00:00");
}

#[test]
fn oid_filter() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "qf_oid_filter";
    seed_set(&agent, &jwt, coll);
    seed(
        &agent,
        &jwt,
        DB,
        coll,
        "q7",
        json!({
            "_id": {"$oid": "65f00000000000000000abcd"},
            "name": "fig", "price": 2, "tag": "fruit",
            "harvested": {"$date": "2026-06-01T00:00:00Z"},
        }),
    );

    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("filter", r#"{"_id":{"$oid":"65f00000000000000000abcd"}}"#)],
    );
    assert_eq!(s, 200, "{b}");
    let docs = b["documents"].as_array().unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["_id"], "65f00000000000000000abcd");
    assert_eq!(docs[0]["name"], "fig");

    // a wrong (valid-looking) oid matches nothing
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("filter", r#"{"_id":{"$oid":"65f00000000000000000abce"}}"#)],
    );
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["documents"].as_array().unwrap().len(), 0);
}

#[test]
fn sort_variants() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "qf_sort_variants";
    seed_set(&agent, &jwt, coll);

    // price asc: NaN bracket first, then numerics by value (1.25 < 3 < 10 < 20 < 25)
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("sort", r#"{"price":1}"#)]);
    assert_eq!(s, 200, "{b}");
    assert_eq!(
        doc_ids(b["documents"].as_array().unwrap()),
        vec!["q5", "q6", "q3", "q1", "q4", "q2"]
    );

    // price desc: reversed, NaN last
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("sort", r#"{"price":-1}"#)]);
    assert_eq!(s, 200, "{b}");
    assert_eq!(
        doc_ids(b["documents"].as_array().unwrap()),
        vec!["q2", "q4", "q1", "q3", "q6", "q5"]
    );

    // multi-key: tag asc, price desc within a tag
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("sort", r#"{"tag":1,"price":-1}"#)],
    );
    assert_eq!(s, 200, "{b}");
    assert_eq!(
        doc_ids(b["documents"].as_array().unwrap()),
        vec!["q6", "q5", "q2", "q4", "q1", "q3"]
    );

    // sort on a field some docs lack: missing sorts as null — first asc, last desc
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("sort", r#"{"opt":1}"#)]);
    assert_eq!(s, 200, "{b}");
    assert_eq!(
        doc_ids(b["documents"].as_array().unwrap()),
        vec!["q2", "q4", "q6", "q1", "q3", "q5"]
    );
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("sort", r#"{"opt":-1}"#)]);
    assert_eq!(s, 200, "{b}");
    // the _id tiebreaker follows the last key's direction (-1), so the
    // missing/null group at the end is ordered by _id descending
    assert_eq!(
        doc_ids(b["documents"].as_array().unwrap()),
        vec!["q5", "q3", "q1", "q6", "q4", "q2"]
    );
}

#[test]
fn limit_semantics() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "qf_limit_semantics";
    seed_set(&agent, &jwt, coll);

    // limit=2: exact page, cursor available, nothing truncated
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("limit", "2")]);
    assert_eq!(s, 200, "{b}");
    assert_eq!(
        doc_ids(b["documents"].as_array().unwrap()),
        vec!["q1", "q2"]
    );
    assert_eq!(b["has_more"], true);
    assert!(b["next_cursor"].is_string());
    assert_eq!(b["truncated"], false);
    assert_eq!(b["limit_applied"], 2);
    assert_eq!(b["count"], 2);

    // limit=0 is rejected
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("limit", "0")]);
    assert_eq!(s, 400, "{b}");
    assert_eq!(err_code(&b), "INVALID_LIMIT");

    // a huge limit caps at the enforced adaptive limit; the whole set still
    // fits, so there is no cursor (has_more=false)
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("limit", "10000")]);
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["truncated"], true);
    assert!(b["limit_applied"].as_u64().unwrap() <= 200);
    assert_eq!(b["count"], 6);
    assert_eq!(b["has_more"], false);
    assert!(b["next_cursor"].is_null());
}

#[test]
fn invalid_filters() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "qf_invalid_filters";

    // malformed JSON -> INVALID_FILTER
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("filter", "not json")]);
    assert_eq!(s, 400, "{b}");
    assert_eq!(err_code(&b), "INVALID_FILTER");

    // non-object filter -> INVALID_FILTER
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("filter", "[1,2]")]);
    assert_eq!(s, 400, "{b}");
    assert_eq!(err_code(&b), "INVALID_FILTER");

    // server-side script operators are refused anywhere (nested too)
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("filter", r#"{"$where":"function(){return true;}"}"#)],
    );
    assert_eq!(s, 400, "{b}");
    assert_eq!(err_code(&b), "INVALID_FILTER");
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[(
            "filter",
            r#"{"$expr":{"$function":{"body":"function(){return true;}","args":[],"lang":"js"}}}"#,
        )],
    );
    assert_eq!(s, 400, "{b}");
    assert_eq!(err_code(&b), "INVALID_FILTER");

    // unknown operator -> client-caused Mongo error -> BAD_REQUEST
    let (s, b) = get_q(
        &agent,
        &jwt,
        DB,
        coll,
        &[("filter", r#"{"x":{"$bogus":1}}"#)],
    );
    assert_eq!(s, 400, "{b}");
    assert_eq!(err_code(&b), "BAD_REQUEST");

    // invalid sort JSON -> INVALID_SORT
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("sort", "not json")]);
    assert_eq!(s, 400, "{b}");
    assert_eq!(err_code(&b), "INVALID_SORT");
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[("sort", r#"{"price":2}"#)]);
    assert_eq!(s, 400, "{b}");
    assert_eq!(err_code(&b), "INVALID_SORT");
}

#[test]
fn no_filter_full_read() {
    let agent = agent();
    let jwt = jwt("main");
    let coll = "qf_no_filter_full_read";
    seed_set(&agent, &jwt, coll);

    // no params: everything back, deterministic _id-asc order
    let (s, b) = get_q(&agent, &jwt, DB, coll, &[]);
    assert_eq!(s, 200, "{b}");
    let docs = b["documents"].as_array().unwrap();
    assert_eq!(docs.len(), 6);
    assert_eq!(b["count"], 6);
    assert_eq!(doc_ids(docs), vec!["q1", "q2", "q3", "q4", "q5", "q6"]);
    // no limit param -> truncated=true, limit_applied = the enforced cap
    assert_eq!(b["truncated"], true);
    assert!(b["limit_applied"].as_u64().unwrap() <= 200);
}
