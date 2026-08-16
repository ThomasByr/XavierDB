//! Auth-flow tests: the /auth contract, identical 401 shapes, malformed
//! bodies, name auto-registration, cookie auth and invalid bearer tokens.
//!
//! Throttle budget: /auth and dashboard login have SEPARATE per-IP throttles
//! and every /auth costs ~5 s of Argon2id — this file performs at most 5
//! fresh logins total (login_ok 1, wrong_token_and_unknown_app 3,
//! new_name_auto_registers 1); everything else uses cached JWTs.

mod common;

use common::*;
use serde_json::json;

#[test]
fn login_ok() {
    ensure_server();
    let agent = agent();
    let (status, body) = auth(&agent, "tester@xdb_tb_main", TOKEN_MAIN);
    assert_eq!(status, 200, "{body}");
    assert!(body["token"].as_str().is_some(), "token present: {body}");
    assert_eq!(body["token_type"], "Bearer");
    assert!(body["expires_in"].is_number());
    assert_eq!(body["expires_in"], 5400);
    assert_eq!(body["identifier"], "tester@xdb_tb_main");
}

#[test]
fn wrong_token_and_unknown_app() {
    ensure_server();
    let agent = agent();
    // wrong token for a real app
    let (s1, b1) = auth(&agent, "tester@xdb_tb_main", "definitely-wrong-token");
    // unknown app id -> verifies against the dummy hash
    let (s2, b2) = auth(&agent, "someone@xdb_tb_nope", TOKEN_MAIN);
    // identifier without '@' -> rejected before any lookup
    let (s3, b3) = auth(&agent, "noapp", TOKEN_MAIN);
    for (s, b) in [(s1, &b1), (s2, &b2), (s3, &b3)] {
        assert_eq!(s, 401, "{b}");
        assert_eq!(b["code"], "UNAUTHORIZED");
        assert_eq!(b["status"], 401);
        assert!(b["error"].is_string());
        // exactly the 3 error-contract keys, nothing else
        assert_eq!(b.as_object().unwrap().len(), 3, "{b}");
    }
    // identical body for every failure kind (no oracle on identity)
    assert_eq!(b1["error"], b2["error"]);
    assert_eq!(b1["error"], b3["error"]);
    // failed attempts do not lock out the valid identity
    let jwt = jwt("main");
    let (status, body) = get_q(&agent, &jwt, DB_SHARED, "seed", &[]);
    assert_eq!(status, 200, "{body}");
    let found = body["documents"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["_id"] == "seed-1");
    assert!(found);
}

#[test]
fn login_malformed() {
    ensure_server();
    let agent = agent();
    // non-JSON body -> 400 with the standard error contract
    let res = agent
        .post(&format!("{}/auth", base()))
        .set("Content-Type", "application/json")
        .send_string("this is not json");
    let (status, body) = finish(res);
    assert_eq!(status, 400, "{body}");
    assert_eq!(err_code(&body), "BAD_REQUEST");
    assert_eq!(err_status(&body), 400);
    // valid JSON missing required fields -> 400 with the standard contract
    for bad in [
        json!({ "token": TOKEN_MAIN }),
        json!({}),
        json!({ "identifier": 7, "token": TOKEN_MAIN }),
    ] {
        let (status, body) = post(&agent, &format!("{}/auth", base()), None, None, Some(&bad));
        assert_eq!(status, 400, "{bad}: {body}");
        assert_eq!(err_code(&body), "BAD_REQUEST");
        assert_eq!(err_status(&body), 400);
    }
}

#[test]
fn new_name_auto_registers() {
    ensure_server();
    let _g = suite_lock().lock().unwrap();
    let agent = agent();
    let cookie = dash_cookie();
    let name = format!("tb_auth_{}", std::process::id());
    let identifier = format!("{name}@{}", APP_MAIN);

    // fetch xdb_tb_main's app-level allow/deny so we can echo them back
    // (perms_save REPLACES allow/deny wholesale per app) — deleting a name
    // must not wipe the fixture's rules
    let (s, p) = dash_get(&agent, &cookie, "/dashboard/api/perms");
    assert_eq!(s, 200);
    let app_entry = p["apps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["app"] == APP_MAIN)
        .expect("xdb_tb_main present in perms");
    let allow = app_entry["allow"].clone();
    let deny = app_entry["deny"].clone();

    // remove any stale copy of this name (idempotent cleanup)
    let del_payload = json!({
        "apps": [{
            "app": APP_MAIN,
            "allow": allow,
            "deny": deny,
            "names": [{ "name": name, "delete": true }],
        }]
    });
    let (s, b) = dash_post(&agent, &cookie, "/dashboard/api/perms", Some(&del_payload));
    assert_eq!(s, 200, "{b}");

    // first login of this name auto-registers it in the permission file
    let (status, body) = auth(&agent, &identifier, TOKEN_MAIN);
    assert_eq!(status, 200, "{body}");

    // ... and it shows up under xdb_tb_main in the dashboard view
    let (s, p) = dash_get(&agent, &cookie, "/dashboard/api/perms");
    assert_eq!(s, 200);
    let app_entry = p["apps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["app"] == APP_MAIN)
        .expect("xdb_tb_main present in perms");
    let listed = app_entry["names"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["name"] == name);
    assert!(listed, "name {name} auto-registered after /auth");

    // remove it again and verify it is gone
    let (s, b) = dash_post(&agent, &cookie, "/dashboard/api/perms", Some(&del_payload));
    assert_eq!(s, 200, "{b}");
    let (s, p) = dash_get(&agent, &cookie, "/dashboard/api/perms");
    assert_eq!(s, 200);
    let app_entry = p["apps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["app"] == APP_MAIN)
        .expect("xdb_tb_main present in perms");
    let gone = !app_entry["names"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["name"] == name);
    assert!(gone, "name {name} removed from perms");
}

#[test]
fn jwt_via_cookie() {
    ensure_server();
    let agent = agent();
    let jwt = jwt("main");
    let (status, body) = call(
        &agent,
        "GET",
        &format!("{}/q/{}/seed", base(), DB_SHARED),
        None,
        Some(&format!("xdb_token={jwt}")),
        None,
    );
    assert_eq!(status, 200, "{body}");
    let found = body["documents"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["_id"] == "seed-1");
    assert!(found);
}

#[test]
fn invalid_bearer() {
    ensure_server();
    let agent = agent();
    let url = format!("{}/q/{}/seed", base(), DB_SHARED);

    // non-JWT garbage
    let (status, body) = get(&agent, &url, Some("garbage"));
    assert_eq!(status, 401, "{body}");
    assert_eq!(body["code"], "UNAUTHORIZED");
    assert_eq!(body["status"], 401);
    assert_eq!(body.as_object().unwrap().len(), 3);

    // a real JWT with one flipped character in the signature segment
    let jwt = jwt("main");
    let sig_start = jwt.rfind('.').expect("jwt has a signature") + 1;
    let mut chars: Vec<char> = jwt.chars().collect();
    let c = chars[sig_start];
    chars[sig_start] = if c == 'a' { 'b' } else { 'a' };
    let flipped: String = chars.into_iter().collect();
    assert_ne!(flipped, jwt);
    let (status, body) = get(&agent, &url, Some(&flipped));
    assert_eq!(status, 401, "{body}");
    assert_eq!(body["code"], "UNAUTHORIZED");
    assert_eq!(body["status"], 401);
}
