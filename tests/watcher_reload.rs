mod common;

// File-watcher + reload endpoints: the authorized_keys.yml watcher (500 ms
// debounce) picks up an appended app entry and a restore of the original
// bytes, and the explicit perms/config reload endpoints answer ok.
//
// The whole file lives under the suite lock. The yml is always restored via
// a Drop guard even when a test panics. One fresh /auth is allowed (the
// wuser login after the watcher picks up the appended app).

use common::*;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const PERMS_FILE: &str = "authorized_keys.yml";

/// Restores the yml bytes on drop (even on panic), then lets the watcher
/// settle for 2 s.
struct RestoreYml(PathBuf, Vec<u8>);

impl Drop for RestoreYml {
    fn drop(&mut self) {
        let _ = std::fs::write(&self.0, &self.1);
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Reads the yml, extracts the `xdb_tb_m2` entry block, and returns it with
/// the app id renamed to `xdb_tb_watch` and its name `u2` renamed to `wuser`
/// (same token_hash, so token tb-m2-secret-token still verifies).
fn watch_app_block(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut i = lines
        .iter()
        .position(|l| *l == "xdb_tb_m2:")
        .expect("xdb_tb_m2 entry present");
    let mut block = String::new();
    block.push_str(lines[i]);
    block.push('\n');
    i += 1;
    while i < lines.len() && lines[i].starts_with(' ') {
        block.push_str(lines[i]);
        block.push('\n');
        i += 1;
    }
    block
        .replacen("xdb_tb_m2:", "xdb_tb_watch:", 1)
        .replacen("    u2:", "    wuser:", 1)
}

#[test]
fn perms_file_watcher_reload() {
    ensure_server();
    let _g = suite_lock().lock().unwrap();
    let agent = agent();
    let cookie = dash_cookie();
    let path = PathBuf::from(PERMS_FILE);
    let original = std::fs::read(&path).expect("read authorized_keys.yml");
    let _restore = RestoreYml(path.clone(), original.clone());

    // (a)+(b)+(c) append the new app entry (same token_hash as xdb_tb_m2)
    let text = String::from_utf8(original.clone()).expect("yml is utf-8");
    let mut new_text = text.clone();
    if !new_text.ends_with('\n') {
        new_text.push('\n');
    }
    new_text.push_str(&watch_app_block(&text));
    std::fs::write(&path, new_text.as_bytes()).expect("write appended yml");
    std::thread::sleep(Duration::from_secs(2)); // watcher debounce 500 ms

    // (e) the watcher reloaded the file: xdb_tb_watch is now listed
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut seen = false;
    while Instant::now() < deadline {
        let (_, body) = dash_get(&agent, &cookie, "/dashboard/api/perms");
        if body["apps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["app"] == "xdb_tb_watch")
        {
            seen = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(seen, "watcher picked up the appended xdb_tb_watch app");

    // (f) the copied token_hash authenticates the new app (one fresh login)
    let (status, body) = auth(&agent, "wuser@xdb_tb_watch", TOKEN_M2);
    assert_eq!(status, 200, "{body}");
    assert!(body["token"].is_string());

    // (g) restore the original bytes; the watcher must pick the restore up
    // on its own (the reload re-stamps the loaded bytes, so the restore is
    // seen as a change again) — no explicit /perms/reload here
    std::fs::write(&path, &original).expect("restore yml");
    std::thread::sleep(Duration::from_secs(2));

    // (h) xdb_tb_watch is gone again (watcher reloaded the restored file)
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut gone = false;
    while Instant::now() < deadline {
        let (_, body) = dash_get(&agent, &cookie, "/dashboard/api/perms");
        if !body["apps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["app"] == "xdb_tb_watch")
        {
            gone = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(gone, "xdb_tb_watch removed by the watcher after restore");
}

#[test]
fn reload_endpoints() {
    ensure_server();
    let _g = suite_lock().lock().unwrap();
    let agent = agent();
    let cookie = dash_cookie();

    let (status, body) = dash_post(&agent, &cookie, "/dashboard/api/perms/reload", None);
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], true);

    let (status, body) = dash_post(&agent, &cookie, "/dashboard/api/config/reload", None);
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], true);
}
