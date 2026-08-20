# Build, run & test — bare metal (any machine; the default when Docker isn't installed)

```bash
npm install && npm run build     # rebuild dashboard TS -> src/assets/app.js (only if TS changed)
# typecheck the dashboard TS (esbuild does NOT typecheck):
#   npx --yes -p typescript tsc --noEmit --strict --target es2020 --lib es2020,dom src/assets/ts/app.ts
cargo build                      # debug; on OSes that lock running executables this
                                 #   fails while the server is running — see restart-ritual.md
cargo test                       # 64 unit + 118 integration tests (tests/); needs a running server
                                 #   + MongoDB — see "Integration battery" below; tests talk to
                                 #   real Mongo unconditionally (XDB_TB_MONGO_URI, default
                                 #   mongodb://localhost:27017; the env-gated unit equivalence
                                 #   test uses XDB_TEST_MONGO_URI, same default)
./target/debug/XavierDB          # from repo root; cwd-relative state files; no CLI args
                                 # (the binary gets a .exe suffix on Windows)
# production-style: cargo build --release → ./target/release/XavierDB
```

Run the dashboard rebuild + server restart as one ritual only if the
dashboard assets changed — see dashboard-rebuild.md. Any src/ change that
needs the test battery: kill → `cargo build --tests` → start → `cargo test`
(restart-ritual.md).

A second server instance can run with env overrides (e.g. `PORT=8443`,
`TLS_CERT_PATH=...`, `TLS_KEY_PATH=...` — env vars override `server.yml`;
`admin.username`/`admin.password_hash` always come from the file) sharing the
same cwd state files.

## Examples (own crate, own lockfile — independent of the server build)

```bash
cargo build --manifest-path examples/Cargo.toml
cargo run --manifest-path examples/Cargo.toml --bin setup_projection -- --admin-user <dashboard-username> --admin-pass <dashboard-password>
cargo run --manifest-path examples/Cargo.toml --bin projection
```

Dashboard username for the setup examples = `server.yml` admin.username (default
`admin`); re-running a setup is idempotent (it refreshes the token hash).
Full contracts: examples.md.

## Integration battery (tests/ — black-box HTTP, needs server + MongoDB up)

118 tests across 13 files (auth_flow, perms_matrix, meta_endpoints, crud_verbs,
edge_data, indexes, query_filters, projection, pagination, dashboard_api,
multi_app, watcher_reload, smoke). Every /auth costs ~5 s Argon2id; /auth and the
dashboard login have SEPARATE per-IP throttles (config 30/min and env 5/min),
so JWTs + the admin cookie are cached in `<temp>/xdb_tb_cache`
and shared across all tests (~0 logins on a warm run; a stale cache
auto-refreshes via probe → re-login).

```bash
bash tests/bootstrap.sh --dash-user <user> --dash-pass '<password>'   # ONE TIME per machine:
                                 #   creates the xdb_tb_* fixture apps via the dashboard perms API,
                                 #   logs in 9 identities, caches JWTs, seeds 8 dbs. Idempotent
                                 #   (skips slow steps when done). Credentials ONLY from the args
                                 #   (or XDB_DASH_USER/XDB_DASH_PASS env) — never from a file.
cargo test                       # everything; binaries run sequentially (~26 s test time)
cargo test --test multi_app      # a single area (auth_flow, perms_matrix, pagination, ...)
```

Fixture world (fully documented in tests/common/mod.rs): 6 apps — xdb_tb_main
(all verbs/dbs), xdb_tb_restricted (GET * minus xdb_tb_secret), xdb_tb_ro (GET
xdb_tb_shared only), xdb_tb_m1 (db globs + name-level deny), xdb_tb_m2
(POST+PATCH only), xdb_tb_m3 (single collection) — plus 8 seeded dbs.
State-mutating tests (perms/config/block) serialize on a suite lock and
restore state; each test uses its own collections; seeding is idempotent.
NOTE: the `count_docs()` GET helper caps at the 200 adaptive limit — for
readbacks of >200 docs use the mongodb driver directly (see the crud_verbs
cap-boundary test).

Machine facts worth knowing:
- Running the battery against the DOCKER stack (API + Mongo in containers,
  local rust tests): no env overrides needed — defaults already point at
  127.0.0.1:8000 / mongodb://localhost:27017 (both published by compose).
  EXCEPTION: `watcher_reload` is expected to FAIL on Docker Desktop (inotify
  does not work over VirtioFS bind mounts — see docker.md); everything else
  is green (verified 108/110, 2026-08-16).
- If `server.yml` has NO `auth.jwt_secret`, the secret is random per restart → cached
  JWTs die on restart; the battery self-heals (probe → 401 → re-login;
  ~9 logins ≈ 45 s once per server start, within the 30/min throttle).
- The mongodb dev-dep resolves to 3.8.0 (create_index takes a single
  IndexModel; count_documents takes Document by value).
- Requires a running MongoDB (default `mongodb://localhost:27017`) —
  install/discover per knowledge/toolchain.md (MongoDB 8.0+ required).
