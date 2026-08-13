//! query — filter operators and extended JSON on GET /q/{db}/{coll}.
//!
//! Prerequisite: run setup_query first (creates the app + rights). Logs in as
//! demo@xdb-query, seeds a small "shop" collection and demonstrates:
//!   - comparison operators ($gte) on Decimal128 prices
//!   - $regex with $options on names
//!   - $exists on an optional field
//!   - $date comparisons on harvested dates
//!   - $oid round-trip: a document inserted with an ObjectId _id comes back
//!     as a plain hex string and can be used in a filter as {"$oid": "..."}
//!   - the extended-JSON output forms ($numberDecimal, $numberDouble for NaN)
//! The filter JSON is passed URL-encoded in the `filter` query parameter.
//!
//! Usage:
//!   cargo run --manifest-path examples/Cargo.toml --bin query -- \
//!       [--token <app-token>] [--app xdb-query] [--name demo] [--base-url http://127.0.0.1:8000]

use serde_json::{json, Value};

const DB: &str = "xdb_query";
const COLL: &str = "items";

fn main() {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(60))
        .build();
    let (base, app, name, token) = args();
    let jwt = login(&agent, &base, &format!("{name}@{app}"), &token);
    println!("authenticated as {name}@{app}\n");

    seed(&agent, &base, &jwt);

    // $gte on Decimal128 prices — cherry (NaN price) never matches, so it is
    // not in the result: comparisons against NaN are always false
    let (status, body) = query(
        &agent,
        &base,
        &jwt,
        r#"{"price":{"$gte":{"$numberDecimal":"1.00"}}}"#,
    );
    println!(
        "price $gte 1.00 ($numberDecimal) -> {status}: {}",
        names(&body)
    );

    // $regex + $options (the two-key form becomes a real regex server-side)
    let (status, body) = query(
        &agent,
        &base,
        &jwt,
        r#"{"name":{"$regex":"^a","$options":"i"}}"#,
    );
    println!("name $regex ^a (i) -> {status}: {}", names(&body));

    // $exists on an optional field
    let (status, body) = query(&agent, &base, &jwt, r#"{"harvested":{"$exists":true}}"#);
    println!("harvested $exists true -> {status}: {}", names(&body));

    // $date comparisons
    let (status, body) = query(
        &agent,
        &base,
        &jwt,
        r#"{"harvested":{"$gte":{"$date":"2026-07-01T00:00:00Z"}}}"#,
    );
    println!(
        "harvested $gte 2026-07-01 ($date) -> {status}: {}",
        names(&body)
    );

    // $oid round-trip: a doc with a real ObjectId _id comes back as a hex
    // string; the same value filters as {"$oid": "..."}
    let (_, fig) = query(&agent, &base, &jwt, r#"{"name":"fig"}"#);
    let id = fig["documents"][0]["_id"].as_str().unwrap().to_string();
    let filter = format!(r#"{{"_id":{{"$oid":"{id}"}}}}"#);
    let (status, body) = query(&agent, &base, &jwt, &filter);
    println!("_id $oid {id} -> {status}: {}", names(&body));

    // raw output forms: dates are ISO strings, Decimal128 and NaN keep their
    // type via extended JSON so re-inserts don't silently change them
    let (_, body) = query(&agent, &base, &jwt, r#"{}"#);
    println!("\nraw documents:");
    pprint(&body["documents"]);
}

fn seed(agent: &ureq::Agent, base: &str, jwt: &str) {
    let docs = [
        json!({"_id":"q-apple",     "name":"apple",     "price":{"$numberDecimal":"1.25"}}),
        json!({"_id":"q-banana",    "name":"banana",    "price":{"$numberDecimal":"0.75"}}),
        json!({"_id":"q-apricot",   "name":"apricot",   "price":{"$numberDecimal":"2.00"}}),
        json!({"_id":"q-carrot",    "name":"carrot",    "price":{"$numberDecimal":"1.10"}}),
        json!({"_id":"q-cucumber",  "name":"cucumber",  "price":{"$numberDecimal":"1.30"},
               "harvested":{"$date":"2026-06-01T08:00:00Z"}}),
        json!({"_id":"q-cherry",    "name":"cherry",    "price":{"$numberDouble":"NaN"},
               "harvested":{"$date":"2026-07-15T10:30:00Z"}}),
        json!({"_id":{"$oid":"65f00000000000000000abcd"},"name":"fig","price":{"$numberDecimal":"0.99"}}),
    ];
    for d in &docs {
        let (status, _) = call(
            agent,
            "POST",
            &format!("{base}/q/{DB}/{COLL}"),
            Some(jwt),
            Some(json!({ "data": d })),
        );
        match status {
            201 | 409 => {} // 409 = already seeded by a previous run
            other => die(&format!("seed insert failed: {other}")),
        }
    }
    println!("seeded {} documents\n", docs.len());
}

fn query(agent: &ureq::Agent, base: &str, jwt: &str, filter: &str) -> (u16, Value) {
    get_q(agent, base, jwt, DB, COLL, &[("filter", filter)])
}

fn get_q(
    agent: &ureq::Agent,
    base: &str,
    jwt: &str,
    db: &str,
    coll: &str,
    params: &[(&str, &str)],
) -> (u16, Value) {
    let mut req = agent
        .get(&format!("{base}/q/{db}/{coll}"))
        .set("Authorization", &format!("Bearer {jwt}"));
    for (k, v) in params {
        req = req.query(k, v);
    }
    finish(req.call())
}

fn names(body: &Value) -> String {
    let names: Vec<String> = body["documents"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|d| d["name"].as_str().unwrap_or("?").to_string())
                .collect()
        })
        .unwrap_or_default();
    format!("{} docs: [{}]", names.len(), names.join(", "))
}

fn login(agent: &ureq::Agent, base: &str, identifier: &str, token: &str) -> String {
    let resp = agent
        .post(&format!("{base}/auth"))
        .send_json(json!({ "identifier": identifier, "token": token }))
        .unwrap_or_else(|e| die(&format!("POST /auth: {e}")));
    let body: Value = resp.into_json().unwrap_or_default();
    body["token"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| die(&format!("no token in /auth response: {body}")))
}

fn call(
    agent: &ureq::Agent,
    method: &str,
    url: &str,
    bearer: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    let mut req = match method {
        "GET" => agent.get(url),
        "POST" => agent.post(url),
        "PUT" => agent.put(url),
        "PATCH" => agent.patch(url),
        "DELETE" => agent.delete(url),
        other => die(&format!("bad method {other}")),
    };
    if let Some(b) = bearer {
        req = req.set("Authorization", &format!("Bearer {b}"));
    }
    let res = match body {
        Some(b) => req.send_json(b),
        None => req.call(),
    };
    finish(res)
}

fn finish(res: Result<ureq::Response, ureq::Error>) -> (u16, Value) {
    match res {
        Ok(resp) => {
            let status = resp.status();
            let v = resp.into_json().unwrap_or(Value::Null);
            (status, v)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let v = resp.into_json().unwrap_or(Value::Null);
            (code, v)
        }
        Err(e) => die(&format!("request failed: {e}")),
    }
}

fn pprint(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

fn args() -> (String, String, String, String) {
    let mut base = "http://127.0.0.1:8000".to_string();
    let mut app = "xdb-query".to_string();
    let mut name = "demo".to_string();
    let mut token = std::env::var("XDB_TOKEN").unwrap_or_else(|_| "demo-token-change-me".into());
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--base-url" => base = it.next().expect("--base-url needs a value"),
            "--app" => app = it.next().expect("--app needs a value"),
            "--name" => name = it.next().expect("--name needs a value"),
            "--token" => token = it.next().expect("--token needs a value"),
            other => die(&format!("unknown option {other}")),
        }
    }
    (base, app, name, token)
}

fn die(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}
