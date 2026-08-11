//! Permissions: authorized_keys.yml parsing, glob matching and the layered
//! resolution model.
//!
//! File format (YAML, one top-level entry per app_id):
//! ```yaml
//! provider1:
//!   token_hash: "$argon2id$v=19$m=65536,t=3,p=4$..."   # PHC string
//!   allow:                      # app-level rights, inherited by every name_id
//!     - actions: [GET, POST]
//!       databases: ["db1", "db*"]
//!       collections: ["*"]
//!   deny: []                    # app-level carve-outs
//!   names:                      # per-name_id overrides (optional)
//!     user1:
//!       allow: [...]
//!       deny: [...]
//! ```
//!
//! Resolution for (name, app, action, db, coll), first match wins:
//!   name.deny -> name.allow -> app.deny -> app.allow -> DENY
//! A name_id with no entries inherits the app permissions entirely.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const ACTIONS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    #[serde(default)]
    pub actions: Vec<String>,
    #[serde(default)]
    pub databases: Vec<String>,
    #[serde(default = "star")]
    pub collections: Vec<String>,
}

fn star() -> Vec<String> {
    vec!["*".to_string()]
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct NameEntry {
    #[serde(default)]
    pub allow: Vec<Rule>,
    #[serde(default)]
    pub deny: Vec<Rule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct AppEntry {
    /// Argon2id PHC hash of the shared app credential. None = auth impossible.
    #[serde(default)]
    pub token_hash: Option<String>,
    #[serde(default)]
    pub allow: Vec<Rule>,
    #[serde(default)]
    pub deny: Vec<Rule>,
    #[serde(default)]
    pub names: BTreeMap<String, NameEntry>,
}

/// Root of authorized_keys.yml: map app_id -> AppEntry.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PermissionsFile {
    pub apps: BTreeMap<String, AppEntry>,
}

impl PermissionsFile {
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        let apps: BTreeMap<String, AppEntry> =
            serde_yaml::from_str(text).map_err(|e| format!("invalid authorized_keys.yml: {e}"))?;
        Ok(Self { apps })
    }

    pub fn to_yaml(&self) -> Result<String, String> {
        serde_yaml::to_string(&self.apps).map_err(|e| format!("serialization failed: {e}"))
    }

    /// Validate that the structure is usable (used before swapping a reload).
    pub fn validate(&self) -> Result<(), String> {
        for (app, entry) in &self.apps {
            if app.is_empty() || app.contains('@') || app.contains(' ') {
                return Err(format!("invalid app_id {app:?}"));
            }
            for (n, rules) in [("allow", &entry.allow), ("deny", &entry.deny)] {
                for r in rules {
                    Self::check_rule(app, n, r)?;
                }
            }
            for (name, nentry) in &entry.names {
                if name.is_empty() || name.contains('@') || name.contains(' ') {
                    return Err(format!("invalid name_id {name:?} for app {app}"));
                }
                for (n, rules) in [("allow", &nentry.allow), ("deny", &nentry.deny)] {
                    for r in rules {
                        Self::check_rule(&format!("{name}@{app}"), n, r)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn check_rule(who: &str, list: &str, r: &Rule) -> Result<(), String> {
        for a in &r.actions {
            if !ACTIONS.contains(&a.as_str()) {
                return Err(format!(
                    "{who}: unknown action {a:?} in {list} (valid: {ACTIONS:?})"
                ));
            }
        }
        if r.actions.is_empty() || r.databases.is_empty() {
            return Err(format!(
                "{who}: {list} rule must list at least one action and one database"
            ));
        }
        for db in &r.databases {
            if db.is_empty() {
                return Err(format!("{who}: empty database pattern in {list}"));
            }
        }
        Ok(())
    }

    /// Does `who` have `action` on (db, coll)?
    pub fn allows(&self, name: &str, app: &str, action: &str, db: &str, coll: &str) -> bool {
        let Some(entry) = self.apps.get(app) else {
            return false;
        };
        let name_layer = entry.names.get(name);
        // layer 1: name-level rules
        if let Some(n) = name_layer {
            if rule_matches(&n.deny, action, db, coll) {
                return false;
            }
            if rule_matches(&n.allow, action, db, coll) {
                return true;
            }
        }
        // layer 2: app-level rules (inheritance)
        if rule_matches(&entry.deny, action, db, coll) {
            return false;
        }
        rule_matches(&entry.allow, action, db, coll)
    }

    /// Databases the caller may GET-list. A database is listable when at
    /// least one effective GET allow-rule matches its name; collections are
    /// filtered afterwards by `listable_collections`.
    pub fn listable_databases(&self, name: &str, app: &str, all_dbs: &[String]) -> Vec<String> {
        all_dbs
            .iter()
            .filter(|db| self.db_accessible(name, app, db))
            .cloned()
            .collect()
    }

    fn db_accessible(&self, name: &str, app: &str, db: &str) -> bool {
        let Some(entry) = self.apps.get(app) else {
            return false;
        };
        let has_get = |rules: &[Rule]| {
            rules.iter().any(|r| {
                r.actions.iter().any(|a| a == "GET")
                    && r.databases.iter().any(|p| glob_match(p, db))
            })
        };
        // a deny covering the whole database hides it from listings; denies of
        // single collections are enforced later by listable_collections
        let denies_whole_db = |rules: &[Rule]| {
            rules.iter().any(|r| {
                r.actions.iter().any(|a| a == "GET")
                    && r.databases.iter().any(|p| glob_match(p, db))
                    && r.collections.iter().any(|p| glob_match(p, "*"))
            })
        };
        // layered, mirroring allows(): name.deny -> name.allow -> app.deny -> app.allow
        if let Some(n) = entry.names.get(name) {
            if denies_whole_db(&n.deny) {
                return false;
            }
            if has_get(&n.allow) {
                return true;
            }
        }
        if denies_whole_db(&entry.deny) {
            return false;
        }
        has_get(&entry.allow)
    }

    /// All collections of `db` the caller may GET.
    pub fn listable_collections(
        &self,
        name: &str,
        app: &str,
        db: &str,
        all_colls: &[String],
    ) -> Vec<String> {
        // whole-database access (pattern that also matches any collection)?
        let whole = self.allows(name, app, "GET", db, "*");
        if whole {
            return all_colls.to_vec();
        }
        all_colls
            .iter()
            .filter(|c| self.allows(name, app, "GET", db, c))
            .cloned()
            .collect()
    }
}

pub fn rule_matches(rules: &[Rule], action: &str, db: &str, coll: &str) -> bool {
    rules.iter().any(|r| {
        r.actions.iter().any(|a| a == action)
            && r.databases.iter().any(|p| glob_match(p, db))
            && r.collections.iter().any(|p| glob_match(p, coll))
    })
}

/// glob-style pattern match ("*" and "?" supported; "**" behaves like "*").
pub fn glob_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" || pattern == "**" {
        return true;
    }
    let pat: Vec<char> = pattern.chars().collect();
    let val: Vec<char> = value.chars().collect();
    // iterative wildcard matching (two-pointer)
    let (mut pi, mut vi) = (0usize, 0usize);
    let (mut star, mut mark) = (usize::MAX, 0usize);
    while vi < val.len() {
        if pi < pat.len() && (pat[pi] == '?' || pat[pi] == val[vi]) {
            pi += 1;
            vi += 1;
        } else if pi < pat.len() && pat[pi] == '*' {
            star = pi;
            mark = vi;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            vi = mark;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == '*' {
        pi += 1;
    }
    pi == pat.len()
}

// ---------------------------------------------------------------------------
// Dashboard helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct EffectiveRule {
    pub source: &'static str, // "name_allow" | "name_deny" | "app_allow" | "app_deny"
    pub actions: Vec<String>,
    pub databases: Vec<String>,
    pub collections: Vec<String>,
}

/// Layered rules with their source, for the dashboard permission viewer.
pub fn effective_rules(
    perms: &PermissionsFile,
    app: &str,
    name: Option<&str>,
) -> Vec<EffectiveRule> {
    let mut out = Vec::new();
    let Some(entry) = perms.apps.get(app) else {
        return out;
    };
    if let Some(n) = name {
        if let Some(nentry) = entry.names.get(n) {
            for r in &nentry.deny {
                out.push(EffectiveRule {
                    source: "name_deny",
                    actions: r.actions.clone(),
                    databases: r.databases.clone(),
                    collections: r.collections.clone(),
                });
            }
            for r in &nentry.allow {
                out.push(EffectiveRule {
                    source: "name_allow",
                    actions: r.actions.clone(),
                    databases: r.databases.clone(),
                    collections: r.collections.clone(),
                });
            }
        }
    }
    for r in &entry.deny {
        out.push(EffectiveRule {
            source: "app_deny",
            actions: r.actions.clone(),
            databases: r.databases.clone(),
            collections: r.collections.clone(),
        });
    }
    for r in &entry.allow {
        out.push(EffectiveRule {
            source: "app_allow",
            actions: r.actions.clone(),
            databases: r.databases.clone(),
            collections: r.collections.clone(),
        });
    }
    out
}

/// Convenience: map of action -> list of "db/collection" pattern pairs,
/// for a compact "what can this identity do" display.

/// Atomically write the permissions YAML to disk. Returns the bytes written.
pub fn persist_perms(state: &crate::state::AppState, yaml: &str) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let path = &state.perms_path;
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp)
            .map_err(|e| format!("cannot create {}: {e}", tmp.display()))?;
        f.write_all(yaml.as_bytes())
            .map_err(|e| format!("write failed: {e}"))?;
        f.sync_all().map_err(|e| format!("fsync failed: {e}"))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("rename failed: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(yaml.as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PermissionsFile {
        PermissionsFile::parse(
            r#"
provider1:
  token_hash: "$argon2id$v=19$m=65536,t=3,p=4$AAAA$BBBB"
  allow:
    - actions: [GET, POST]
      databases: ["db1", "db*"]
      collections: ["*"]
    - actions: [DELETE]
      databases: ["db2"]
      collections: ["coll_b"]
  deny:
    - actions: [DELETE]
      databases: ["db1"]
      collections: ["secret_*"]
  names:
    user1:
      allow:
        - actions: [GET]
          databases: ["db1"]
          collections: ["coll_a"]
"#,
        )
        .unwrap()
    }

    #[test]
    fn parse_ok() {
        let p = sample();
        assert!(p.validate().is_ok());
        assert!(p.apps.contains_key("provider1"));
    }

    #[test]
    fn inheritance() {
        let p = sample();
        // user2 has no rules -> inherits app permissions
        assert!(p.allows("user2", "provider1", "GET", "db1", "anything"));
        assert!(p.allows("user2", "provider1", "DELETE", "db2", "coll_b"));
        // app deny beats app allow
        assert!(!p.allows("user2", "provider1", "DELETE", "db1", "secret_x"));
    }

    #[test]
    fn name_rules_win() {
        let p = sample();
        // user1 explicit: only GET on db1/coll_a (from name layer)
        assert!(p.allows("user1", "provider1", "GET", "db1", "coll_a"));
        // name rules don't grant DELETE; app rules do -> still allowed via inheritance
        assert!(p.allows("user1", "provider1", "DELETE", "db2", "coll_b"));
        // name deny can carve out
        let mut p2 = p.clone();
        p2.apps
            .get_mut("provider1")
            .unwrap()
            .names
            .get_mut("user1")
            .unwrap()
            .deny
            .push(Rule {
                actions: vec!["DELETE".into()],
                databases: vec!["db2".into()],
                collections: vec!["coll_b".into()],
            });
        assert!(!p2.allows("user1", "provider1", "DELETE", "db2", "coll_b"));
    }

    #[test]
    fn globs() {
        assert!(glob_match("db*", "db1"));
        assert!(glob_match("db*", "db"));
        assert!(!glob_match("db*", "xdb1"));
        assert!(glob_match("*_log", "access_log"));
        assert!(glob_match("coll?", "collA"));
        assert!(!glob_match("coll?", "collAB"));
        assert!(glob_match("*", ""));
        assert!(glob_match("a*b*c", "aXbYc"));
        assert!(!glob_match("a*b*c", "aXbY"));
    }

    #[test]
    fn unknown_app_denied() {
        let p = sample();
        assert!(!p.allows("user1", "nope", "GET", "db1", "coll_a"));
    }

    #[test]
    fn db_accessible_respects_deny() {
        // whole-db deny hides the database from listings
        let mut p = sample();
        p.apps.get_mut("provider1").unwrap().deny.push(Rule {
            actions: vec!["GET".into()],
            databases: vec!["db2".into()],
            collections: vec!["*".into()],
        });
        assert!(p.db_accessible("user2", "provider1", "db1"));
        assert!(!p.db_accessible("user2", "provider1", "db2"));
        // name-level deny wins over name/app allow
        let mut p2 = sample();
        p2.apps
            .get_mut("provider1")
            .unwrap()
            .names
            .get_mut("user1")
            .unwrap()
            .deny
            .push(Rule {
                actions: vec!["GET".into()],
                databases: vec!["db1".into()],
                collections: vec!["*".into()],
            });
        assert!(!p2.db_accessible("user1", "provider1", "db1"));
        // collection-scoped deny keeps the db listable (listable_collections
        // filters the denied collection afterwards)
        let mut p3 = sample();
        p3.apps.get_mut("provider1").unwrap().deny.push(Rule {
            actions: vec!["GET".into()],
            databases: vec!["db1".into()],
            collections: vec!["coll_a".into()],
        });
        assert!(p3.db_accessible("user2", "provider1", "db1"));
        assert!(!p3.allows("user2", "provider1", "GET", "db1", "coll_a"));
    }

    #[test]
    fn bad_action_rejected() {
        let p = PermissionsFile::parse(
            "app1:\n  allow:\n    - actions: [FLY]\n      databases: [db1]\n",
        );
        assert!(p.is_ok()); // YAML is well-formed...
        let p = p.unwrap();
        assert!(p.validate().is_err()); // ...but validation catches the bad action
    }
}
