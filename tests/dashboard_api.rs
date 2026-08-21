mod common;

// Dashboard API integration tests: session guard, login, perms merge/delete
// semantics, unknown-field tolerance, config get/save/undo, block/unblock
// (app + name), app weight, logs, databases, export, metrics shape.
//
// One fresh dashboard login is allowed (the wrong-password test); everything
// else uses dash_cookie(). State-mutating tests hold suite_lock() and
// restore state (unblock, re-add apps, restore weights/config).

use common::*;
use serde_json::json;

// Best-effort removal of a temp app on panic (state restore).
struct TempAppGuard {
    app: String,
    cookie: String,
}

impl Drop for TempAppGuard {
    fn drop(&mut self) {
        let agent = agent();
        let _ = dash_post(
            &agent,
            &self.cookie,
            "/dashboard/api/perms",
            Some(&json!({ "apps": [{ "app": self.app, "delete": true }] })),
        );
    }
}

#[test]
fn cookie_required() {
    ensure_server();
    let agent = agent();
    let (status, body) = get(&agent, &format!("{}/dashboard/api/metrics", base()), None);
    assert_eq!(status, 401);
    assert_eq!(err_code(&body), "UNAUTHORIZED");
    assert_eq!(err_status(&body), 401);
    let (status, body) = dash_get(&agent, "xdb_admin=garbage", "/dashboard/api/metrics");
    assert_eq!(status, 401);
    assert_eq!(err_code(&body), "UNAUTHORIZED");
}

#[test]
fn login_wrong_password() {
    ensure_server();
    let agent = agent();
    let (status, body) = post(
        &agent,
        &format!("{}/dashboard/api/login", base()),
        None,
        None,
        Some(&json!({ "username": "no-such-user", "password": "wrong-password" })),
    );
    assert_eq!(status, 401);
    assert_eq!(err_code(&body), "UNAUTHORIZED");
}

#[test]
fn perms_get_shape() {
    ensure_server();
    let agent = agent();
    let cookie = dash_cookie();
    let (status, body) = dash_get(&agent, &cookie, "/dashboard/api/perms");
    assert_eq!(status, 200);
    assert!(body["version"].is_number());
    let apps = body["apps"].as_array().expect("apps array");
    let main = apps
        .iter()
        .find(|a| a["app"] == APP_MAIN)
        .expect("xdb_tb_main present");
    assert_eq!(main["token_set"], true);
    let allow = main["allow"].as_array().expect("allow array");
    assert!(!allow.is_empty(), "xdb_tb_main allow is non-empty");
    let eff = main["effective"].as_array().expect("effective array");
    assert!(!eff.is_empty());
    for r in eff {
        assert!(r["source"].is_string());
        assert!(r["actions"].is_array());
        assert!(r["databases"].is_array());
        assert!(r["collections"].is_array());
    }
    let names = main["names"].as_array().expect("names array");
    assert!(
        names.iter().any(|n| n["name"] == "tester"),
        "names contain tester"
    );
}

#[test]
fn perms_merge_add_delete() {
    ensure_server();
    let _g = suite_lock().lock().unwrap();
    let agent = agent();
    let cookie = dash_cookie();
    let app = format!("xdb_tb_tmp_{}", std::process::id());
    let _cleanup = TempAppGuard {
        app: app.clone(),
        cookie: cookie.clone(),
    };

    let rule = json!({ "actions": ["GET"], "databases": ["xdb_tb_extra"], "collections": ["*"] });
    let (status, body) = dash_post(
        &agent,
        &cookie,
        "/dashboard/api/perms",
        Some(&json!({ "apps": [{ "app": app, "allow": [rule], "deny": [], "names": [] }] })),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], true);

    // GET shows the app; allow was replaced wholesale (exactly one rule)
    let (_, body) = dash_get(&agent, &cookie, "/dashboard/api/perms");
    let entry = body["apps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["app"] == app)
        .expect("temp app present");
    let allow = entry["allow"].as_array().unwrap();
    assert_eq!(allow.len(), 1);
    assert_eq!(allow[0]["actions"], json!(["GET"]));
    assert_eq!(allow[0]["databases"], json!(["xdb_tb_extra"]));
    assert_eq!(allow[0]["collections"], json!(["*"]));
    assert_eq!(entry["deny"].as_array().unwrap().len(), 0);

    // delete:true removes the app
    let (status, body) = dash_post(
        &agent,
        &cookie,
        "/dashboard/api/perms",
        Some(&json!({ "apps": [{ "app": app, "delete": true }] })),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], true);
    let (_, body) = dash_get(&agent, &cookie, "/dashboard/api/perms");
    assert!(
        !body["apps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["app"] == app),
        "temp app gone after delete:true"
    );
}

#[test]
fn perms_unknown_fields_ignored() {
    ensure_server();
    let _g = suite_lock().lock().unwrap();
    let agent = agent();
    let cookie = dash_cookie();

    // a GET snapshot POSTed back verbatim must succeed (roundtrip stability)
    let (_, snap) = dash_get(&agent, &cookie, "/dashboard/api/perms");
    let (status, body) = dash_post(&agent, &cookie, "/dashboard/api/perms", Some(&snap));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], true);

    // bogus fields at the top level and inside an app entry are ignored
    let m2 = snap["apps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["app"] == APP_M2)
        .expect("xdb_tb_m2 present")
        .clone();
    let mut m2_bogus = m2;
    m2_bogus["bogus"] = json!(1);
    let (status, body) = dash_post(
        &agent,
        &cookie,
        "/dashboard/api/perms",
        Some(&json!({ "bogus": 1, "apps": [m2_bogus] })),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], true);

    // content unchanged (the version bumps on every save, so compare apps)
    let (_, after) = dash_get(&agent, &cookie, "/dashboard/api/perms");
    assert_eq!(
        after["apps"], snap["apps"],
        "apps unchanged by bogus fields"
    );
}

#[test]
fn config_get_shape() {
    ensure_server();
    let agent = agent();
    let cookie = dash_cookie();
    let (status, body) = dash_get(&agent, &cookie, "/dashboard/api/config");
    assert_eq!(status, 200);
    assert!(body["version"].is_number());
    assert!(body["config"]["global"].is_object());
    assert!(body["config"]["global"]["jwt_token_lifetime_minutes"].is_number());
    assert!(body["config"]["rate_limit"].is_object());
    assert!(body["history"].is_array());
    assert!(body["undo_available"].is_boolean());
    assert!(body["redo_available"].is_boolean());
}

#[test]
fn config_post_and_undo() {
    ensure_server();
    let _g = suite_lock().lock().unwrap();
    let agent = agent();
    let cookie = dash_cookie();

    let (_, snap) = dash_get(&agent, &cookie, "/dashboard/api/config");
    let orig = snap["config"]["health"]["cache_ttl_seconds"]
        .as_u64()
        .expect("cache_ttl_seconds");
    let new = if orig == 7 { 3 } else { 7 };
    let mut cfg = snap["config"].clone();
    cfg["health"]["cache_ttl_seconds"] = json!(new);
    let (status, body) = dash_post(
        &agent,
        &cookie,
        "/dashboard/api/config",
        Some(&json!({ "config": cfg })),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["config"]["health"]["cache_ttl_seconds"], json!(new));

    let (status, body) = dash_post(&agent, &cookie, "/dashboard/api/config/undo", None);
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], true);

    let (_, after) = dash_get(&agent, &cookie, "/dashboard/api/config");
    assert_eq!(
        after["config"]["health"]["cache_ttl_seconds"].as_u64(),
        Some(orig),
        "undo restored the original cache_ttl_seconds"
    );
}

#[test]
fn block_unblock_app() {
    ensure_server();
    let _g = suite_lock().lock().unwrap();
    let agent = agent();
    let cookie = dash_cookie();

    // warm the u2 JWT while the app is NOT blocked (a stale cache would try a
    // fresh login, which must not happen while blocked)
    let u2 = jwt("u2");

    let (status, body) = dash_post(
        &agent,
        &cookie,
        "/dashboard/api/block",
        Some(&json!({ "id": APP_M2 })),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], true);

    // /auth for a name under the blocked app -> 403 BLOCKED (fresh login)
    let (status, body) = auth(&agent, "u2@xdb_tb_m2", TOKEN_M2);
    assert_eq!(status, 403, "{body}");
    assert_eq!(err_code(&body), "BLOCKED");

    // /q with a valid JWT -> 403 BLOCKED (per-request check)
    let (status, body) = get_q(&agent, &u2, DB_SHARED, "blk_probe", &[]);
    assert_eq!(status, 403, "{body}");
    assert_eq!(err_code(&body), "BLOCKED");

    let (status, body) = dash_post(
        &agent,
        &cookie,
        "/dashboard/api/unblock",
        Some(&json!({ "id": APP_M2 })),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], true);

    // u2 is an ingester: a POST insert into xdb_tb_shared works again
    let coll = format!("blk_app_{}", std::process::id());
    let (status, body) = post_q(
        &agent,
        &u2,
        DB_SHARED,
        &coll,
        &json!({ "data": { "v": 1 } }),
    );
    assert_eq!(status, 201, "{body}");
}

#[test]
fn block_unblock_name() {
    ensure_server();
    let _g = suite_lock().lock().unwrap();
    let agent = agent();
    let cookie = dash_cookie();

    // warm both JWTs before blocking reader2
    let reader = jwt("reader");
    let reader2 = jwt("reader2");
    let coll = format!("blk_name_{}", std::process::id());

    let (status, body) = dash_post(
        &agent,
        &cookie,
        "/dashboard/api/block",
        Some(&json!({ "id": "reader2@xdb_tb_ro" })),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], true);

    let (status, body) = get_q(&agent, &reader2, DB_SHARED, &coll, &[]);
    assert_eq!(status, 403, "{body}");
    assert_eq!(err_code(&body), "BLOCKED");

    // same app, different name -> unaffected (name-level isolation)
    let (status, body) = get_q(&agent, &reader, DB_SHARED, &coll, &[]);
    assert_eq!(status, 200, "{body}");

    let (status, body) = dash_post(
        &agent,
        &cookie,
        "/dashboard/api/unblock",
        Some(&json!({ "id": "reader2@xdb_tb_ro" })),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], true);

    let (status, body) = get_q(&agent, &reader2, DB_SHARED, &coll, &[]);
    assert_eq!(status, 200, "{body}");
}

#[test]
fn app_weight() {
    ensure_server();
    let _g = suite_lock().lock().unwrap();
    let agent = agent();
    let cookie = dash_cookie();

    let (_, snap) = dash_get(&agent, &cookie, "/dashboard/api/config");
    let orig = snap["config"]["rate_limit"]["weights"][APP_M2]
        .as_f64()
        .unwrap_or(1.0);

    let (status, body) = dash_post(
        &agent,
        &cookie,
        "/dashboard/api/app_weight",
        Some(&json!({ "id": APP_M2, "weight": 0.5 })),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["weight"], json!(0.5));

    let (_, after) = dash_get(&agent, &cookie, "/dashboard/api/config");
    assert_eq!(after["config"]["rate_limit"]["weights"][APP_M2], json!(0.5));

    // restore the original weight
    let (status, body) = dash_post(
        &agent,
        &cookie,
        "/dashboard/api/app_weight",
        Some(&json!({ "id": APP_M2, "weight": orig })),
    );
    assert_eq!(status, 200, "{body}");
    let (_, back) = dash_get(&agent, &cookie, "/dashboard/api/config");
    let back_w = back["config"]["rate_limit"]["weights"][APP_M2]
        .as_f64()
        .unwrap_or(1.0);
    assert!(
        (back_w - orig).abs() < 1e-9,
        "weight restored (got {back_w}, want {orig})"
    );
}

#[test]
fn logs_databases_export() {
    ensure_server();
    let agent = agent();
    let cookie = dash_cookie();

    let (status, body) = dash_get(&agent, &cookie, "/dashboard/api/logs");
    assert_eq!(status, 200);
    let lines = body["lines"].as_array().expect("lines array");
    assert!(!lines.is_empty(), "log ring is non-empty");

    let (status, body) = dash_get(&agent, &cookie, "/dashboard/api/databases");
    assert_eq!(status, 200);
    assert_eq!(body["unavailable"], false);
    let dbs = body["databases"].as_array().expect("databases array");
    assert!(!dbs.is_empty());
    for d in dbs {
        assert!(d["name"].is_string());
        assert!(d["collections"].is_array());
    }
    assert!(
        dbs.iter().any(|d| d["name"] == DB_SHARED),
        "xdb_tb_shared listed"
    );

    let (status, body) = dash_get(&agent, &cookie, "/dashboard/api/config/export");
    assert_eq!(status, 200);
    assert!(
        body["global"].is_object(),
        "export body is the config document"
    );
    assert!(body["rate_limit"].is_object());
    let text = serde_json::to_string(&body).unwrap();
    assert!(
        text.contains("permission_file"),
        "export JSON contains config content"
    );
}

#[test]
fn metrics_shape() {
    ensure_server();
    let agent = agent();
    let cookie = dash_cookie();
    let (status, body) = dash_get(&agent, &cookie, "/dashboard/api/metrics");
    assert_eq!(status, 200);
    assert!(body["ts"].is_number());
    assert!(body["system"]["cpu_pct"].is_number());
    assert!(body["system"]["mem_pct"].is_number());
    assert!(body["system"]["uptime_s"].is_number());
    assert!(body["config"]["cfg_version"].is_number());
    assert!(body["cursors"]["count"].is_number());
    assert!(body["cursors"]["list"].is_array());

    let apps = body["apps"].as_array().expect("apps array");
    let main = apps
        .iter()
        .find(|a| a["app"] == APP_MAIN)
        .expect("xdb_tb_main present");
    assert!(main["blocked"].is_boolean());
    assert!(main["weight"].is_number());
    assert!(main["rps"].is_number());
    assert!(main["p50_ms"].is_number());
    assert!(main["limit"].is_number() || main["limit"].is_null());
    assert!(main["breakdown"].is_object() || main["breakdown"].is_null());
    assert!(main["rps_history"].is_array());
    assert!(main["names"].is_array());
}
