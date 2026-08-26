---
name: build-run-test
description: "Build, run & test the XavierDB Rust server. Docker-first flow: xdb-compose.sh up (API + MongoDB) then battery.sh run (local cargo test against the published ports), with a bare-metal xdb-restart.sh fallback ritual only when Docker is broken. Use when compiling, launching, or running the integration test battery."
---

# Build, run & test — Docker-first (bare metal only as fallback)

> **Scripts:** `battery.sh` (`bootstrap [--dash-user U] [--dash-pass P] | run |
> single <area> | all`) and `build.sh` (host build — fallback/low-level checks
> only). Compose ops: `skills/docker/xdb-compose.sh` (**the default**). Prefer
> the scripts over hand-typed commands; defaults overridable via `XDB_*` env
> (see `.agents/settings/defaults.sh`).

## DEFAULT flow — tests run against the Docker stack (always try this first)

```bash
xdb-compose.sh up          # API + MongoDB in containers (MongoDB is ALWAYS Docker)
battery.sh run             # local cargo test against 127.0.0.1:8000 /
                           # mongodb://localhost:27017 — both published by compose, no env overrides
```

- Any src/ change that needs the test battery: `xdb-compose.sh up` (rebuilds
  the image) → `battery.sh run`.
- Dashboard asset changes: `xdb-dashboard.sh bundle` then `xdb-compose.sh up`
  (the SPA is embedded at compile time — the image must be rebuilt) — see
  dashboard-rebuild/SKILL.md.
- EXCEPTION: `watcher_reload` fails against the Docker-Desktop stack (no
  inotify over VirtioFS bind mounts) — everything else is green; see
  docker/SKILL.md.

## FALLBACK — bare metal, ONLY when Docker fails

See `skills/docker-fallback/SKILL.md` — activating criteria (broken daemon,
Docker Desktop filesystem quirks, low-level host Rust checks) + its
`xdb-restart.sh` `kill|build|start|test` subcommands. MongoDB STAYS in Docker
(`docker compose up -d mongodb` or `docker run -p 27017:27017 mongo:8.0`).

```bash
npm install && npm run build     # rebuild dashboard TS -> src/assets/app.js (only if TS changed)
# typecheck the dashboard TS (esbuild does NOT typecheck):
#   npx --yes -p typescript tsc --noEmit --strict --target es2020 --lib es2020,dom src/assets/ts/app.ts
xdb-restart.sh build             # cargo build --tests; fails while the server runs on some OSes
xdb-restart.sh start             # ./target/debug/XavierDB (XavierDB.exe on Windows); own command
xdb-restart.sh test              # 64 unit + 118 integration tests; needs the server + MongoDB up
                                 # (XDB_TB_MONGO_URI default mongodb://localhost:27017; the
                                 #  env-gated equivalence unit tests use XDB_TEST_MONGO_URI)
# production-style: cargo build --release → ./target/release/XavierDB
```

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
Full contracts: examples/SKILL.md.

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
- **Default (Docker stack — API + Mongo in containers, local rust tests):** no
  env overrides needed — defaults already point at 127.0.0.1:8000 /
  mongodb://localhost:27017 (both published by compose). EXCEPTION:
  `watcher_reload` is expected to FAIL on Docker Desktop (inotify does not
  work over VirtioFS bind mounts — see docker/SKILL.md); everything else is
  green (verified 108/110, 2026-08-16).
- **Fallback (server on bare metal, docker issues):** MongoDB STILL via Docker
  (`docker compose up -d mongodb` / `docker run -p 27017:27017 mongo:8.0`),
  server via the docker-fallback ritual (skills/docker-fallback/SKILL.md).
- Live-config flakiness (observed 2026-08-24): if the live `config` has an
  aggressive `rate_limit.tick_seconds` (e.g. 1 instead of the default 5),
  battery load can transiently shrink the adaptive limit mid-run → flaky
  `multi_app` failures (`adaptive_limit_cap`, `concurrent_writers`; enforced
  limit dips far below the expected 200/40). The limit recovers at
  `growth_rate` per tick — a rerun passes. Not a code regression.
- If `server.yml` has NO `auth.jwt_secret`, the secret is random per restart → cached
  JWTs die on restart; the battery self-heals (probe → 401 → re-login;
  ~9 logins ≈ 45 s once per server start, within the 30/min throttle).
- The mongodb dev-dep resolves to 3.8.0 (create_index takes a single
  IndexModel; count_documents takes Document by value).
- Requires MongoDB 8.0+ — ALWAYS launched with Docker per knowledge/toolchain.md
  (compose `mongodb` service, or `docker run -p 27017:27017 mongo:8.0`).
