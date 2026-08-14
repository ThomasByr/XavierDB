# AGENTS.md — XavierDB

Standalone guide for working in this repository. Everything an agent needs to
understand the project, find files, run things, and know what to look for.
Keep this file updated as the project changes — it is the canonical agent
reference. The notebook pages (see §9) hold TODOs and small remarks that are
too fine-grained for this file; when they conflict, AGENTS.md wins, but update
it so it stays accurate and self-contained.

---

## 0. Agent rules (read first)

1. **Python: always `uv run python` (or `uv run --with <pkg> python -c "..."`),
   never a system `python`.** A system interpreter may be missing, be an
   unusable stub, or be the wrong version — `uv` fetches a managed interpreter
   on demand (verified: Python 3.14 via uv 0.11.x). If `uv` is absent, install
   it with the official installer (see §8 for detection + install). Example:
   `uv run --with pyyaml python -c "import yaml"`.
2. **Never `git commit`.** The user handles all commits (their GPG signing is
   misconfigured; commits fail anyway). Make changes in the working tree only.
3. **Docker is optional.** If `docker` and the compose plugin are not on PATH
   (`command -v docker && docker --version && docker compose version`), run
   bare metal (§4.1). The compose/Docker setup (§4.2) is UNVERIFIED — it has
   never been run anywhere yet.
4. **Some OSes refuse to overwrite a running executable** — `cargo build`
   fails until the server is killed (on the dev machine the error is
   "Accès refusé"). Restart ritual (each step a SEPARATE shell command; the
   start must be its own command — a shell that times out can kill the
   disowned server: keep commands short and use `--max-time` on curls):
   1. stop the server by process name — POSIX: `pkill XavierDB`; Windows:
      `taskkill //F //IM XavierDB.exe` (plain `taskkill /F /IM XavierDB.exe`
      in non-MSYS shells). Verify down (`curl /health` fails).
   2. `cargo build --tests` (rebuilds the server binary AND keeps the
      test-mode fingerprints fresh)
   3. start detached (own command), e.g.
      `./target/debug/XavierDB >> /tmp/xdb.log 2>&1 & disown`
      (the binary is `XavierDB.exe` on Windows).
   **Trap:** never run plain `cargo build` between `cargo build --tests` and
   `cargo test` — the normal-mode build re-invalidates the test-mode bin
   fingerprint (mongodb is built as two separate units, one per graph) and
   `cargo test` then tries to relink the server binary → lock. `cargo test
   --no-run` has the same effect. The only clean sequence is:
   kill → `cargo build --tests` → start → `cargo test`. `cargo check` is safe
   while the server runs.
5. Consult the notebooks (§9) for TODOs and small remarks before starting
   work; after work, update AGENTS.md (and the notebook pages) so both stay
   current. AGENTS.md must remain standalone and ready at all times.
6. `.env` may be awkward to touch from some shells (a protected path on the
   dev machine) — read it via `read`/`cat` with care; `PASSWORD_HASH` is
   single-quoted in the file.
7. Credentials are machine-local: read them from `.pi/notes/credentials.md`
   (gitignored — never commit or copy them into docs/AGENTS.md). See §8.1 for
   how to obtain or regenerate them on a fresh machine.

---

## 1. What this is

A small, fast HTTP server (Rust, axum 0.8, tokio, mongodb driver) that exposes
a **MongoDB database through a REST API**: per-client authentication (JWT),
granular permissions (`authorized_keys.yml`), adaptive per-app document
limits, a binary config file with undo/redo history, and an embedded
Material-3-ish admin dashboard SPA (no JS libraries, no external fonts).
Edition 2024. No Python/Node at runtime (Node only at build time for
the dashboard TypeScript). Cross-platform Rust; no OS-specific code at runtime.

Routes (top level, `src/main.rs`):

```
POST /auth                                 client login -> JWT (+ HttpOnly cookie)
GET|POST|PUT|PATCH|DELETE /q/<db>/<coll>   MongoDB proxy (JWT-protected)
GET  /ls                                   list databases the caller may read (?db=<db> -> collections)
/dashboard/ + /dashboard/api/*             admin dashboard (login-protected SPA + JSON API)
GET  /health                               cached health document (public)
```

---

## 2. Repository layout

```
XavierDB/
├── README.md                    # quick start (Docker-first) + bare metal in <details>
├── AGENTS.md                    # this file
├── docs/                        # the full documentation set
│   ├── ADMIN_GUIDE.md           #   dashboard views, ops, troubleshooting, sparse dashboard API section
│   ├── API_REFERENCE.md         #   client API only + verified JS/Python examples
│   └── CONFIGURATION.md         #   config file fields, adaptive-limit formula, perms format
├── compose.yaml                 # 2 services: xavierdb (MongoDB) + api; api mounts repo over /app
├── Dockerfile                   # node stage (esbuild) + single-stage rust:1-slim-bookworm build/run
├── .dockerignore                # excludes .env/config/config.bak*/authorized_keys.yml/target/node_modules/.git/.pi
├── .gitignore                   # /target, .env, authorized_keys.yml, config, config.bak*, node_modules/
├── Cargo.toml / Cargo.lock      # Rust workspace (axum, tokio, mongodb, rustls/aws-lc-sys, argon2, notify, serde_yaml…)
├── package.json / package-lock.json   # esbuild devDependency only (dashboard TS -> JS)
├── examples/                      # standalone crate: 8 runnable client examples (see examples/README.md)
│   ├── Cargo.toml / Cargo.lock    #   own deps (ureq + serde_json only), own lockfile
│   └── src/bin/                   #   per example: setup_<name>.rs (dashboard API) + <name>.rs (client API)
├── tests/                         # integration battery — BLACK-BOX HTTP tests, need a running server+Mongo (§4.1)
│   ├── common/mod.rs              #   shared helpers: fixture world docs, cached JWTs/admin cookie, HTTP wrappers, suite lock
│   ├── bootstrap.sh               #   one-time fixture bootstrap (idempotent; dashboard creds from env or credentials.md)
│   └── auth_flow.rs, crud_verbs.rs, dashboard_api.rs, edge_data.rs, meta_endpoints.rs,
│       multi_app.rs, pagination.rs, perms_matrix.rs, projection.rs, query_filters.rs,
│       smoke.rs, watcher_reload.rs   # 110 tests, ~30 s full run
├── .env.example                 # documented env template (copy to .env)
├── authorized_keys.yml.example  # documented permissions template
├── src/
│   ├── main.rs                  # startup, env, watchers (config/perms hot reload), router, /health
│   ├── auth.rs                  # JWT issue/verify, /auth, Argon2id, throttle
│   ├── routes_q.rs              # /q proxy + /ls handler, per-request perms check, cursor pagination
│   ├── dbq.rs                   # MongoDB queries, cursor encode/decode (keyset), listing cursors
│   ├── perms.rs                 # authorized_keys.yml parsing, globs, layered first-match-wins evaluation
│   ├── config.rs                # ConfigFile: XDB1 magic + crc32 + bincode, atomic writes, backups, history/undo
│   ├── routes_admin.rs          # all /dashboard/api/* endpoints (~681 lines)
│   ├── metrics.rs               # adaptive limit engine, rate/EMA computation, pressure
│   ├── state.rs                 # AppState, ClientStats (delta-based counters), sessions
│   ├── tls.rs                   # optional TLS, cert hot reload
│   ├── error.rs                 # {error, code, status} contract
│   └── assets/
│       ├── assets.rs            # serves embedded SPA files under /dashboard/ no-cache
│       ├── index.html           # static shell (login + app shell)  [static]
│       ├── styles.css           # design tokens + all styles              [static]
│       ├── app.js               # GENERATED by esbuild — never hand-edit
│       └── ts/app.ts            # dashboard SPA source (~2050 lines TS) — edit here
├── .env                         # local (bare-metal) env; gitignored; NOT in Docker image
├── config / config.bak*         # binary settings + backups; gitignored; runtime state
├── authorized_keys.yml          # app credentials + permissions; gitignored; runtime state
├── target/                      # build output (excluded everywhere)
└── node_modules/                # npm deps (excluded everywhere)
```

### What the image vs. the repo contains (Docker)

`.dockerignore` excludes `.env`, `config`, `config.bak*`, `authorized_keys.yml`
from the build context. `COPY . .` (build stage) lands in `/build`; the
runtime stage's `/app` workdir is empty until compose mounts the repo root
over it (`.:/app`), so the repo files ARE the container's state files: the
container reads/writes the same `.env`/`config`/`config.bak`/`authorized_keys.yml` as
bare metal. Secrets never enter image layers (mount ≠ image). Mongo data stays
in a named bind mount `${HOME}/data/xavier-mongo-db`.

---

## 3. Runtime state files — the "config world"

All are **cwd-relative**: the server reads/writes them relative to its working
directory (repo root bare metal; `/app` = repo mount in Docker).

| file | format | purpose | hot reload? |
|---|---|---|---|
| `.env` | dotenv | HOST, PORT, MONGODB_URI, MAX_WORKERS, MAX_INSERT_BATCH, TLS paths, USERNAME, PASSWORD_HASH (single-quoted!), JWT_SECRET, LOG_FILES (1–10), LOG_SIZE_MB (1–20) | **No** — dotenvy reads at process start; restart the process (`docker compose restart api` in Docker) |
| `config` | XDB1 magic + crc32 + bincode | all tunables + history/redo/blocked | **Yes** — file watcher (500ms debounce) AND `/dashboard/api/config/reload` |
| `config.bak…` | same | automatic backup rotation (MAX_BACKUPS=5) on save; fallback on corruption | n/a |
| `authorized_keys.yml` | YAML | app credentials (Argon2id hashes) + layered permissions | **Yes** — file watcher (500ms debounce) + `/perms/reload` |
| `xavierdb.log…` | text | rotating server log: current + `xavierdb.log.1..N` (env `LOG_FILES`/`LOG_SIZE_MB`, defaults 5 × 10 MB); the Logs tab reads these files — no in-memory ring | n/a (env-only settings) |

Startup behavior (`main.rs`):
- `.env` missing → written from `include_str!("../.env.example")` (template
  compiled INTO the binary, not read from the repo). If `PASSWORD_HASH` is
  blank/unparseable → generate a strong password, Argon2id-hash it into `.env`
  (single-quoted), **print plaintext once** to stdout/logs (`docker compose
  logs api` on Docker).
- `config` missing/corrupt → try `config.bak*` chain → else defaults, and a
  default file is **written to disk** (`config.rs::load_from_disk`).
- `authorized_keys.yml` missing → "starting with no permissions",
  `PermissionsFile::default()` (everything 403). **Watcher gotcha:** the file
  watcher cannot attach to a non-existent file ("file may not exist yet") — if
  you create the file after startup, restart the server.

Watcher details (`main.rs::start_watchers`): notify crate, 500ms debounce;
self-writes are skipped via `last_config_written`/`last_perms_written` byte
comparison; a successful watcher reload re-stamps the loaded bytes, so a
restore of a file to the server's previous write is detected as a change
again; invalid files → keep previous state + error log.

---

## 4. Build & run

### 4.1 Bare metal (any machine; the default when Docker isn't installed)

```bash
npm install && npm run build     # rebuild dashboard TS -> src/assets/app.js (only if TS changed)
# typecheck the dashboard TS (esbuild does NOT typecheck):
#   npx --yes -p typescript tsc --noEmit --strict --target es2020 --lib es2020,dom src/assets/ts/app.ts
cargo build                      # debug; on OSes that lock running executables this
                                 #   fails while the server is running (§0.4)
cargo test                       # 44 unit + 110 integration tests (tests/); needs a running server
                                 #   + MongoDB — see "Integration battery" below; tests talk to
                                 #   real Mongo unconditionally (XDB_TB_MONGO_URI, default
                                 #   mongodb://localhost:27017; the env-gated unit equivalence
                                 #   test uses XDB_TEST_MONGO_URI, same default)
./target/debug/XavierDB          # from repo root; cwd-relative state files; no CLI args
                                 # (the binary gets a .exe suffix on Windows)

# Examples (own crate, own lockfile — independent of the server build):
cargo build --manifest-path examples/Cargo.toml
cargo run --manifest-path examples/Cargo.toml --bin setup_projection -- --admin-user <dashboard-username> --admin-pass <dashboard-password>
cargo run --manifest-path examples/Cargo.toml --bin projection
```

Dashboard username for the setup examples = `.env` USERNAME (default
`admin`); re-running a setup is idempotent (it refreshes the token hash).
A second server instance can run with env overrides (e.g. `PORT=8443`)
sharing the same cwd state files — fine for read-only testing; stop it by PID
(`lsof -i :8443` on POSIX, `netstat -ano | grep :8443` on Windows), never by
process name (that kills every instance).

#### Integration battery (tests/ — black-box HTTP, needs server + MongoDB up)

110 tests across 12 files (auth_flow, perms_matrix, meta_endpoints, crud_verbs,
edge_data, query_filters, projection, pagination, dashboard_api, multi_app,
watcher_reload, smoke). Every /auth costs ~5 s Argon2id and shares a 30/min
per-IP throttle, so JWTs + the admin cookie are cached in `<temp>/xdb_tb_cache`
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

Lock gotcha: on OSes that lock running executables, `cargo test` relinks the
bin test harness whenever src/ or Cargo.toml changed — kill the server first,
`cargo build --tests`, restart the server, then `cargo test` (incremental
test runs don't relink). Never run plain `cargo build` between
`cargo build --tests` and `cargo test` (it re-dirties the test-mode
fingerprint — see §0.4).

- Requires a running MongoDB (default `mongodb://localhost:27017`) — install/
  discover per §8 (MongoDB 8.0+ required).
- Production-style: `cargo build --release` → `./target/release/XavierDB`.

### 4.2 Docker (optional — the compose setup is UNVERIFIED, never run anywhere yet)

```bash
docker compose up --build -d     # builds API image + starts MongoDB + API (incremental: layer cache)
docker compose watch             # rebuilds image on ./Cargo.toml or ./src changes
docker compose build --no-cache api   # force a full rebuild when the cache is suspect
docker compose logs api          # first-run dashboard password is printed here
```

- `compose.yaml`: api has **no `image:` key** (build-only; with both, compose
  would tag the build and clobber the official `rust:1-slim-bookworm` tag).
  Mongo healthcheck uses a marker-file hack (retries 100 × 5s); api healthcheck
  is plain `curl -fsS http://localhost:8000/health`. Compose `environment:`
  (`HOST=0.0.0.0`, `MONGODB_URI=mongodb://xavierdb:27017`) wins over `.env`
  (dotenvy never overrides existing env vars).
- `develop.watch` rebuilds on source change; the repo mount does NOT hot-reload
  Rust code (binary lives at `/usr/local/bin/XavierDB`, outside `/app`).
- On a LINUX Docker host, container writes to the repo (default config
  creation, config.bak rotation, `.env` bootstrap) are root-owned — you may
  need sudo to edit them; Desktop-style mounts (Docker Desktop) are
  transparent. Fix if it bites: `user: "${UID}:${GID}"` (Linux hosts only).

### 4.3 Untested areas (be honest about these)

All of the following are UNVERIFIED — development so far happened without
Docker and without a real browser:

- Full Docker build (aws-lc-sys in container, cmake/perl/pkg-config install),
  healthcheck behavior, `${HOME}` interpolation on the user's Docker host,
  notify-watcher behavior inside a container.
- The dashboard UI in a real browser (API contracts verified via curl and
  jsdom repros only — see §9 gaps).

---

## 5. Architecture

### 5.1 Auth

- `POST /auth` `{identifier, token}` → validates identifier against
  `authorized_keys.yml` + Argon2id-verifies the shared token → returns
  `{token, token_type:"Bearer", expires_in:5400, identifier}` + `Set-Cookie:
  xdb_token` (HttpOnly; Secure under TLS). 401 bad creds, 403 BLOCKED, 429
  throttle.
- JWT: HS256, secret = env `JWT_SECRET` or random-per-start; lifetime from
  `config.global.jwt_token_lifetime_minutes` (default 90). Expired/malformed →
  401 with a 5s leeway; reason swallowed. Client loop: on 401 re-auth, on 403
  do NOT re-auth.
- Blocked ids (in `config.blocked`) → 403 BLOCKED at `/auth`.
- **The app token is shared by every name under an app** (one Argon2id hash
  per app in authorized_keys.yml): any holder can /auth as ANY `name@app` —
  existing or not (new names are auto-added to the yml on first login). The
  name_id is a permission-routing label, not a credential; name-level rules
  separate identities within the app only. Each name needs its own /auth for
  its own JWT (sub = exact name) — see notebook `xavierdb-auth-model`.
- Dashboard sessions: in-memory DashMap (`xdb_admin` cookie, Path=/dashboard,
  TTL `config.auth.session_ttl_hours` default 24) — **restart = re-login**.

### 5.2 Permissions (`authorized_keys.yml`)

- Structure: `apps: {app_id: {token_hash, allow: [rules], deny: [rules], names:
  {name: {allow, deny}}}}`. Rule = `{actions, databases, collections}`.
  Globs `*` and `?`. Template: `authorized_keys.yml.example`.
- **Layered, first-match-wins**: name.deny → name.allow → app.deny →
  app.allow → deny.
- New names are auto-added to the yml on `/auth` — this rewrites the whole
  file (reformatted, comments lost, same as dashboard saves); permission file
  hot-reloads.
- Client path perms are re-checked live on EVERY `/q/` request. Apps with no
  rules inherit nothing — they get the layered defaults.
- Dashboard rewrites of the file **lose comments** (known limit).
- Watcher loss window (known limit): an external hand-edit of
  authorized_keys.yml or the config file can be silently lost if the server
  writes its own copy within the ~500 ms watcher debounce (the self-write
  byte-stamp then suppresses the reload).

### 5.3 `/q/<db>/<coll>` proxy (routes_q.rs)

- GET: params `filter`/`sort`/`projection` = URL-encoded JSON (extended JSON ok), `limit`,
  `cursor`. `projection` = top-level include/exclude object (values 1/0/true/false;
  mixed styles rejected except `_id`; `_id:0` ok; `{}` no-op; dotted/`$` keys →
  400 INVALID_PROJECTION). Sort fields + `_id` are force-added to the Mongo
  projection and stripped from the output (union+strip, dbq.rs
  `projection_document`/`projection_strip_fields`/`bson_to_json_projected`) so
  keyset pagination and the array-sort guard keep working — cursors are
  projection-independent. Response `{documents:[…], next_cursor, has_more, truncated,
  limit_applied, count}`. Server caps `limit` at the app's adaptive limit.
- POST: `{filter?, data}` — no filter = insert (201): `data` object →
  `insert_one` (`{inserted_count:1, inserted_id}`); `data` array → `insert_many`
  (`{inserted_count:n, inserted_ids:[…]}` in input order, cap
  `state.max_insert_batch` — env `MAX_INSERT_BATCH`, default 1000 (main.rs,
  routes_q.rs `MAX_INSERT_BATCH`), must be ≥ 1, also published top-level in
  `/health`; empty/
  non-object/dup-`_id`-within-batch → 400 with nothing inserted; dup against
  existing data → 409 with ordered semantics — docs before the dup remain).
  With filter = update (200 `{matched_count, modified_count}`).
  **`data` is auto-wrapped in `$set` server-side** (routes_q.rs:480) —
  clients send plain `data: {field: value}`, NOT `{$set:…}`. Batch inserts
  count their document count into per-client rate accounting (rps) — a
  1000-doc batch reads as 1000 work units (routes_q.rs `record_request`).
  **Upsert-many is on PATCH** (see below): POST batches are insert-only.
- PUT = update (404 if 0 matched). PATCH = upsert (200 updated / 201
  inserted) with `{filter, data: object}`; **upsert-many** with `{data:
  [objects]}` (no filter) — always 200 `{matched_count, modified_count,
  inserted_count, upserted_count, inserted_ids, upserted_ids}` (ids in input
  order). Per element: has `_id` → upserting UpdateOne on `{_id}` with $set
  merge (filter carries the `_id`); no `_id` → InsertOne. One PATCH authorize
  covers the whole batch. Same
  `batch_to_docs` validation as POST (empty/non-object/dup-`_id`-within/
  over-cap → 400, nothing applied), `{filter, data: array}` → 400 "batch
  upsert takes no filter"; conflicts (existing `_id`/unique index) → 409
  ordered semantics, docs before the failing element remain (dbq.rs
  `bulk_upsert` via driver `Client::bulk_write`, requires MongoDB 8.0+;
  batch size counts into rate accounting). DELETE `{filter}` → `{deleted_count}` (404 if 0).
- Cursor pagination: keyset, opaque base64url cursor
  `{v, db, coll, sort:[[field,dir]..], last:[canonical-extjson..]}` with `_id`
  tiebreaker; listing cursors use plain JSON-string values and require
  `last.len() == sort.len()` (dbq.rs:361 — a mismatch breaks decoding:
  "wrong listing cursor"). Mixed-type/missing sort fields paginate correctly:
  each keyset column gets a same-type `$gt/$lt` branch plus a `$type`
  bracket-fallback branch (null boundaries skip the `$gt/$lt` branch to avoid
  re-serving nulls; NaN boundaries continue with `$gte: -Inf` and a `{f: NaN}`
  branch — NaN sorts first ascending on MongoDB 8). ARRAY sort values are
  refused with 400 when a page needs continuation (element-wise array sort is
  not representable in a keyset cursor).
- Filter hardening: server-side script operators `$where`/`$function` are
  rejected (400 INVALID_FILTER) everywhere a filter is parsed (GET/POST/PUT/
  PATCH/DELETE). Mongo client-caused command errors (bad regex, malformed
  shapes, validation) map to 400, duplicate keys to 409 CONFLICT.
- Output forms: non-finite doubles → `{"$numberDouble":"NaN"}`, Decimal128 →
  `{"$numberDecimal":"…"}` (plain strings/`null` would silently change the
  type on re-insert). `$regex`+`$options` (two-key object) converts to a real
  regex; `$timestamp` requires non-negative `t`/`i`.

### 5.4 `/ls` (replaced `/q/dataset` — no alias, that route now 404s; `dataset` is NOT reserved)

- `GET /ls` → `{databases: ["a","b"], next_cursor, has_more, limit_applied}` —
  FLAT name strings, permission-filtered, cursor-paginated over dbs only.
- `GET /ls?db=X` → `{db:"X", collections:[...]}`; 404 when X doesn't exist;
  403 when X exists but the caller has no access.
- Handler: `routes_q::list_visible`, registered on the top-level router.

### 5.5 Adaptive limit (metrics.rs)

- Per-app document limit, re-derived every tick (default 5s):
  `lat_err = max(0,(p50−target)/target)`, `pressure = max(0,(cpu−60)/40,
  (mem−70)/30)`, `shrink = 1/(1+K_l·lat_err+K_p·pressure)`; internal limit ×=
  shrink if <1 else × growth_rate, clamped [min_limit, max_limit]; enforced =
  `round(internal · multiplier · weight).clamp(min,max)`. Internal STARTS at
  max_limit on first tick. Per-app `weight` in `config.rate_limit.weights`
  (0.1–10, dashboard-editable, default 1.0).
- Rates are delta-based: `ClientStats.last_total` cumulative counters, EMA
  smoothing (alpha = `config.rate_limit.ema_alpha`), decay to 0 when idle;
  history = 120 samples per tick. Both app AND name keys get rates/sparklines;
  adaptive limit is app-only (`key[4..]` strips the `app:` prefix).
- Requests over the limit: first page + `next_cursor` (client must paginate;
  the server never loads a huge set into RAM).

### 5.6 Config file (config.rs)

- `XDB1` magic + crc32 + len + bincode; unknown version refused → backup fallback.
  Atomic writes (tmp + fsync + rename); backups `config.bak`, `config.bak.2`,
  … rotate (MAX_BACKUPS=5, chain is real: oldest dropped, rest shifted, fresh
  copy — verified by test). History capped at 10k snapshots `{ts, desc, path,
  snapshot, by}`; **snapshots are FLAT (no history/redo inside)** — nesting
  them would double the file size on every mutation; undo/redo/revert rebuild
  the entry list from metadata. API returns history NEWEST-first and
  `revert {index}` takes the NEWEST-FIRST display position (0 = newest).
- Sanitization (routes_admin): POST/import/revert clamp every field; the
  invariants min_limit ≤ 10 000 and min_limit ≤ max_limit always hold (the
  metrics loop clamps with both, min > max would panic it). `load_from_disk`
  additionally raises max_limit to min_limit on load.
- Key fields (defaults): global{jwt_token_lifetime_minutes=90,
  permission_file="authorized_keys.yml"}, rate_limit{min=1, max=200,
  multiplier=1.0, target=50, pressure_sens=1.5, latency_sens=1.0, growth=1.15,
  tick=5, ema=0.2, weights{}}, health{ttl=5}, dashboard{poll=2, smoothing=5,
  log_level="info", theme="system"}, auth{per_ip=30, session_ttl_h=24}, blocked[], history[],
  redo[].

### 5.7 Health

- `GET /health` (public, cached, default TTL 5s):
  `{status:"ok|degraded|unhealthy", checked_at_ms, next_refresh_seconds,
  compute_latency_ms, qps, max_insert_batch, app:{status, uptime_s,
  p50_latency_ms, total_requests, active_cursors}, mongodb:{reachable,
  ping_latency_ms, error}}` — 200 only when ok, else 503. `max_insert_batch`
  is the insert-batch cap (MAX_INSERT_BATCH env), static per process — the
  battery reads it from here so cap-boundary tests work with custom values.

### 5.8 Dashboard

- Embedded SPA (`include_str!` at compile time, served no-cache under
  `/dashboard/`), hash-routed, 4 pages: `#/overview | #/clients | #/config |
  #/logs`. Permissions/rate-limit pages were removed (2026-08 rework).
- TS source `src/assets/ts/app.ts` (~2050 lines) → esbuild → `src/assets/app.js`
  (generated, never hand-edit). No JS libs, no external fonts.
- Full dashboard API surface (all under `/dashboard/api/*`, `xdb_admin`
  session cookie; errors same `{error, code, status}` shape): login/logout/
  session, metrics (big poll payload), block/unblock, app_weight, perms
  GET/POST(full-merge)/reload, databases, config GET/POST/undo/redo/reload/
  reset/revert/export/import, logs (rotating FILES on disk, env-configured
  LOG_FILES/LOG_SIZE_MB — no in-memory ring; ?limit&before paging + app/name
  facets; every console line incl. eprintln/panics). See §6.
- Config tab: EXPLICIT save — slider edits alone don't persist (a page
  reload discards them); an amber "unsaved changes" dirty pill is pinned to
  the card title line (`margin-left:auto` inside the flex `h3` — never in the
  buttons row), Save is disabled while clean, and an in-flight `configSaving`
  guard prevents double POSTs.
- Logs box colors are theme-aware `--logs-*` tokens, defined in ALL THREE
  theme blocks (`:root` light, `prefers-color-scheme: dark`, forced
  `[data-theme="dark"]`) — any new theme-aware token must land in all three.
- Browser-behavior debugging without a browser: a jsdom repro drives the
  SERVED bundle (fetch `/dashboard/` index.html + app.js — re-fetch after
  EVERY rebuild, the embed is compile-time), stubs fetch/matchMedia, and
  simulates clicks; a local copy lives in the user's temp dir (pattern in
  notebook `xavierdb-dashboard-ui`).
- UI architecture details (badge permission editor, detached scopes, weight
  popover, config slider form): kept in notebook `xavierdb-dashboard-ui`; the
  essential contracts are in §6.

---

## 6. API reference (condensed — full details in docs/API_REFERENCE.md and docs/ADMIN_GUIDE.md)

### Client API

| route | auth | behavior |
|---|---|---|
| `POST /auth` | public (throttled) | login → JWT + cookie (see 5.1) |
| `GET /q/{db}/{coll}` | Bearer or cookie | query: `filter`/`sort`/`projection` URL-encoded JSON, `limit`, `cursor`; keyset pagination |
| `POST /q/{db}/{coll}` | Bearer | insert (no filter) / update (filter) — data auto-`$set` |
| `PUT /q/{db}/{coll}` | Bearer | update, 404 if 0 matched |
| `PATCH /q/{db}/{coll}` | Bearer | upsert (200 updated / 201 inserted); array `data` = upsert-many (200) |
| `DELETE /q/{db}/{coll}` | Bearer | `{filter}` → `{deleted_count}`, 404 if 0 |
| `GET /ls` | Bearer | flat list of listable dbs; `?db=X` → collections |
| `GET /health` | public | health doc; 200 ok / 503 otherwise |

Errors: `{error, code, status}`; codes BAD_REQUEST/INVALID_FILTER/INVALID_SORT/
INVALID_LIMIT/INVALID_CURSOR (400), UNAUTHORIZED (401), FORBIDDEN/BLOCKED (403),
NOT_FOUND (404), CONFLICT (409, duplicate key), TOO_MANY_REQUESTS (429),
INTERNAL_ERROR (500), UNAVAILABLE (503). Messages sanitized (paths,
IPv4/IPv6 scrubbed; bare hostnames/host:port are NOT — they're deployment
config). Client-caused Mongo command errors (bad regex, malformed shapes,
validation) map to 400; duplicate keys → 409.

### Dashboard API (condensed)

- `POST /dashboard/api/login` `{username, password}` → `{"ok":true}` + cookie
  `xdb_admin` (Path=/dashboard, HttpOnly, SameSite=Strict, Max-Age follows
  `auth.session_ttl_hours`). Throttle SHARED with client /auth (per-IP,
  default 30/min). Argon2id verify runs on the blocking pool; unknown
  usernames verify against a fixed dummy hash (no timing oracle).
- `POST /dashboard/api/logout` / `GET /dashboard/api/session` →
  `{"username":"…"}`; sessions are in-memory (restart = re-login).
- `GET /dashboard/api/metrics` — big poll payload: `{ts, qps, config:
  {poll_seconds, theme, graph_smoothing, cfg_version, perms_version,
  health_ttl_seconds, multiplier}, system:{cpu_pct, mem_pct, mem_used_mb,
  mem_total_mb, disk_pct, disk_used_mb, disk_total_mb, net_rx_kbps,
  net_tx_kbps, uptime_s, ts_ms}, health,
  apps:[{app, blocked, weight, rps, p50_ms, limit, breakdown:{internal,
  enforced, lat_err, pressure, shrink, p50_ms, rate, updated_ms},
  rps_history, names:[{name, id:"n@app", blocked, rps, p50_ms,
  total_requests, last_seen_ms, rps_history}]}], cursors:{count, list}}`.
  Apps = perms-file apps ∪ live-seen, sorted; zero-stats rows still appear;
  cursors sorted by last_used_ms DESC, truncated to 30. UI polls every
  `poll_seconds`; perms drift via `config.perms_version != permsData.version`.
- `POST /dashboard/api/block` / `unblock` `{id}` (bare `app` or `name@app`,
  1..=130 chars) → mutates `config.blocked` with history.
- `POST /dashboard/api/app_weight` `{id, weight}` — 0.1..=10, snapped to 0.1.
- `GET /dashboard/api/perms` → `{version, apps:[{app, token_set, allow, deny,
  effective:[{source, actions, databases, collections}], names:[...]}]}`.
  `POST /dashboard/api/perms` — MERGE semantics: only listed apps touched;
  app-level allow/deny REPLACED wholesale per app; `delete:true` removes
  app/name; `set_token` (min 8 chars) rehashes. Unknown JSON fields ignored —
  a GET snapshot can be POSTed verbatim. `POST /perms/reload` re-reads yml.
- `GET /dashboard/api/config` → `{version, config, history (NEWEST-first),
  undo_available, redo_available}`; `POST /config` sanitizes/clamps (see
  notebook `xavierdb-dashboard-api` for exact ranges); undo/redo/reload
  (fallback to config.bak on corruption, returns `warning`)/revert
  `{index}`/reset/export (JSON attachment)/import. undo/redo/reload/reset are
  POST-with-NO-body (`{}` → 400); log_level changes hot-apply (no restart).
- `GET /dashboard/api/logs` → `{lines:[{seq, raw, level, logger, app, name}],
  total, apps, names, loggers, retention:{files, size_mb, path}}` — reads the
  ROTATING LOG FILES (xavierdb.log + .1..N, env LOG_FILES/LOG_SIZE_MB, no
  in-memory ring); `?limit=N` (0 = all), `?before=<seq>` load-older paging;
  `apps`/`names`/`loggers` = facets from a bounded scan (last 2000 lines).
  Logs SURVIVE restarts, and `seq` (a global line number seeded by a startup
  scan) stays stable across restarts AND rotations.
- `GET /dashboard/api/databases` → `{databases:[{name, collections}],
  unavailable}` — admin-only, unfiltered (client-side equivalent: `/ls`).

---

## 7. Docs index

| file | contents |
|---|---|
| `README.md` (repo root) | quick start (Docker-first), bare metal `<details>`, route table, "why this shape", Files table |
| `docs/API_REFERENCE.md` | client API only + verified JS/Python examples in `<details>`; dashboard → points to ADMIN_GUIDE.md#dashboard-api |
| `docs/ADMIN_GUIDE.md` | dashboard views (4-view), ops, troubleshooting; sparse dashboard API section |
| `docs/CONFIGURATION.md` | config fields table (verified against config.rs defaults), adaptive-limit formula, perms format |
| `authorized_keys.yml.example` | documented permissions template |

---

## 8. Environment & toolchain discovery

The server is cross-platform Rust; Python and Node are build/dev-time only.
On a fresh machine, discover what's already installed (POSIX shell syntax —
MSYS-style shells like git-bash accept it; PowerShell/cmd need their own
equivalents):

| tool | when needed | detect | install if missing |
|---|---|---|---|
| Rust (cargo + rustc) | always | `command -v cargo && cargo --version` | rustup (https://rustup.rs); edition 2024 needs rustc ≥ 1.85 (developed on 1.97) |
| Node + npm | build-time only (esbuild compiles the dashboard TS) | `command -v node && node --version` | nodejs.org or a version manager (nvm/fnm/volta) |
| uv | dev scripts (managed Python on demand) | `command -v uv && uv --version` | official installer: https://docs.astral.sh/uv/ |
| python3 | dev scripts ONLY via uv — a system interpreter is not required | `command -v python3` (informational) | none — uv fetches managed interpreters |
| MongoDB (mongod) | runtime + integration battery | `command -v mongod && mongod --version` | official MongoDB packages; **8.0+ required** |
| Docker + compose plugin | optional (compose deployment, §4.2) | `command -v docker && docker --version && docker compose version` | docker.com; WITHOUT Docker, run bare metal (§4.1) |

Required versions / facts:
- **MongoDB 8.0+** — PATCH upsert-many uses the driver's `bulk_write` (new
  bulkWrite command, no legacy path; §5.3). The dev machine runs 8.0.12.
- **Rust**: edition 2024 → rustup latest stable is fine (developed on 1.97).
- **Node**: only at build time (esbuild 0.28); any modern LTS is fine.
  **Python**: only dev scripts; always `uv run python` (§0.1). **mongosh** is
  optional — shell access only; neither the server nor the battery needs it.
- A bare-metal run needs only two things: `mongod` reachable at
  `mongodb://localhost:27017` (override with `MONGODB_URI` in `.env`) and
  `./target/debug/XavierDB` started from the repo root (cwd-relative state
  files, no CLI args).
- On the machine where this project was developed there is no Docker and no
  system python — bare metal + uv are the norm. Machine-level details
  (portable-mongod layout, shell quirks, ops commands) live in the notebook
  `xavierdb-local-run`; actual credentials in §8.1.

### 8.1 Credentials (machine-local)

Actual credentials are NOT in this file — they are machine-local and live in
`.pi/notes/credentials.md` (gitignored via `.pi/.gitignore`; read it when you
need them). That file also holds dev-environment notes (test dbs, log paths).
If it is missing (new machine, wiped notes), obtain or regenerate them as
follows:

- **Dashboard password** — the plaintext is printed EXACTLY ONCE, at first
  bootstrap, in the server log: bare metal = the terminal/stdout the server
  was started with (e.g. `/tmp/xdb.log`); Docker = `docker compose logs api`.
  `.env` only ever holds the Argon2id `PASSWORD_HASH` (not reversible). To
  force a fresh password: blank `PASSWORD_HASH` in `.env` (or copy
  `.env.example` over `.env`) and restart the server — a new password is
  generated and printed once. `USERNAME` comes from `.env` (default `admin`).
- **Client app tokens** (`identifier` = `name@app`, shared secret token) —
  `authorized_keys.yml` stores only the Argon2id `token_hash`, so the
  plaintext is NOT recoverable. If lost, reset via the dashboard (Clients
  view → add app / perms editor → set token, min 8 chars) or rewrite the yml
  entry with a freshly hashed token. To hash one:
  `uv run --with argon2-cffi python -c "..."` (Argon2id PHC) — verify it
  against the SERVER (swap `token_hash` in authorized_keys.yml → watcher
  reload → /auth), not against the library (argon2-cffi's verify has been
  observed broken in some environments).
- **TLS certs** — paths are `TLS_CERT_PATH`/`TLS_KEY_PATH` in `.env`;
  regenerate with openssl (self-signed is fine for dev).
- **MongoDB** — URI in `.env` (`MONGODB_URI`, default
  `mongodb://localhost:27017`); install/discovery per §8; dev-machine
  portable-mongod details in notebook `xavierdb-local-run`.

---

## 9. Notebook pages (TODOs, small remarks, history)

The pi notebook holds the fine-grained, session-by-session knowledge. Consult
it for TODOs and small remarks; promote anything durable into AGENTS.md when
it becomes load-bearing. Pages:

- `xavierdb-agents-rewrite` — this machine-agnostic rewrite: plan, constraints, scrub inventory, absorption map
- `xavierdb-auth-model` — auth Q&A: name_id is a routing label, shared app token, JWT claims, timing equalization
- `xavierdb-build` — build facts, client API verification, perms test cycle, rate-computation fix history
- `xavierdb-compose` — compose/Dockerfile decisions, runtime-state mechanics, first-run behavior, caveats
- `xavierdb-dashboard-api` — complete dashboard API reference (exact shapes from routes_admin.rs)
- `xavierdb-dashboard-ui` — dashboard UI architecture (edit points, badge model, dirty indicator, jsdom harness, known gaps)
- `xavierdb-dataset-route` — /ls rename history, cursor bug fix
- `xavierdb-docs` — docs restructure, AGENTS.md, credentials layout
- `xavierdb-examples` — examples/ crate: scope decisions, verified perms/dashboard-API facts, per-example contracts (DONE 2026-08-13)
- `xavierdb-insert-many` — insert-many on POST /q: contract, driver 3.8 facts, tests (DONE 2026-08-15)
- `xavierdb-local-run` — local run: portable MongoDB setup, ops commands, machine conventions
- `xavierdb-projection` — GET /q projection: design spec + implementation record (DONE 2026-08-13), verified cursor/keyset mechanics, union+strip scheme, latent dotted-sort-key bug
- `xavierdb-review` — the 3-round review campaign: per-finding verdicts, fixes, test augmentation
- `xavierdb-test-battery` — tests/ integration battery: fixture world, bootstrap, verified behaviors + spec corrections (DONE 2026-08-15)
- `xavierdb-upsert-many` — upsert-many on PATCH /q: contract, driver 3.8 facts, tests (DONE 2026-08-15)

---

## 10. Known limits & open items

Known limits (by design, not bugs):
- Admin sessions in-memory → restart = re-login.
- Dashboard rewrite of authorized_keys.yml loses its comments.
- Keyset pagination refuses (400) to continue past a page whose sort field
  contains an **array** value — MongoDB's element-wise array sort cannot be
  represented in a keyset cursor; silent loss/loops would be worse. NaN/±Inf
  sort values ARE handled (NaN sorts first ascending on MongoDB 8).
- `/auth` and dashboard login share the per-IP throttle. The throttle keys on
  the peer socket IP — `X-Forwarded-For` is deliberately NOT trusted (no proxy
  in the deployment; the header is client-controlled). Window is a fixed
  wall-clock minute: up to 2× the limit can pass across a minute boundary.
- `/auth` + dashboard login: Argon2id verify runs on the tokio blocking pool
  (never on async workers); unknown apps/usernames verify against a fixed
  dummy PHC so response timing doesn't reveal whether an identity exists;
  blocked ids are checked before the hash (403 regardless of token). All
  auth failures return the identical `UNAUTHORIZED` body.

Known gaps / things to check:
- **GET projection: IMPLEMENTED (2026-08-13)** — `projection` param (JSON object, INVALID_PROJECTION 400), union+strip scheme keeps the keyset cursor correct (Mongo always sees sort fields + `_id`; client sees only requested fields). Dotted/nested projection keys and `$`-operators rejected (top-level only, v1). See notebook `xavierdb-projection`.
- **Verified live by the battery (2026-08-14)** — behaviors worth knowing:
  - Include-only projections STRIP `_id` unless explicitly requested: `{name:1}` → docs have only `name`; `{name:1,_id:1}` keeps it. `{_id:0}` alone returns everything except `_id` (FIXED 2026-08-14 — it previously collapsed to `{}`).
  - Dots are valid in COLLECTION names (`bad..name` OK) — 400 only for dots in the db segment. MongoDB 8.0.12 also ACCEPTS `$`-prefixed field names in stored documents (they round-trip literally).
  - Extraction failures (malformed or missing-field JSON bodies, malformed query strings) → **400 `{error, code:"BAD_REQUEST", status:400}`** (FIXED 2026-08-14 — previously axum's plain-text rejections, incl. 422 for missing fields, leaked through). `filter=%zz` decodes leniently → INVALID_FILTER (not a query rejection); `limit=abc` → BAD_REQUEST via the custom extractor.
  - Missing/null sort values sort BEFORE NaN ascending (Mongo 8 order: null < NaN < numbers). `$gte` on a Decimal128 matches int/double values too (cross-type).
  - Watcher: a reload re-stamps the loaded bytes, so a byte-identical restore of authorized_keys.yml IS picked up automatically (FIXED 2026-08-14 — it previously required an explicit `/perms/reload`).
  - `truncated:true` + `limit_applied` = enforced cap when the client requested more than the adaptive limit; `next_cursor` only appears when the set was actually cut.
  - Insert-many (2026-08-15): `data` as array → `insert_many` (cap 1000, empty/non-object element/dup-`_id`-within-batch → 400 with NOTHING inserted; dup against existing data → 409 with ordered semantics, docs before the dup remain). Driver 3.8 facts the battery verified: `insert_many` write failures arrive as `ErrorKind::InsertMany(InsertManyError)` (NOT `WriteFailure::WriteError` — error.rs needed a dedicated arm) and `InsertManyResult.inserted_ids` is a `HashMap<usize, Bson>` (NOT BTreeMap — dbq.rs sorts by index so `inserted_ids` keeps input order). Batch size counts into per-client rate accounting (rps).
- **Array `_id` (`{"_id":[]}`) maps to 500** — Mongo error code 53 ("_id" cannot be an array) is not in the client-code list in error.rs; arguably a client error → 400. Pre-existing for single writes, consistent for the insert/upsert batch arms (flagged 2026-08-15, not changed).
- **Dotted sort keys (`{"a.b":1}`) paginate incorrectly** — pre-existing latent bug (found 2026-08-11 during projection design; code-verified, not live-verified): `bson::Document::get` is an exact top-level lookup (no dotted resolution), so `make_next_cursor` reads a Null boundary and the array-sort guard goes blind → wrong pagination on collections sorted by nested fields. Fix = `get_path` helper + equivalence-test guard; treat as separate follow-up.
- Dashboard UI not yet browser-tested (API contracts verified via curl and
  jsdom repros; see §5.8) — a first browser pass may reveal weight-popover
  overflow, legend wrap, slider feel.
- Docker setup never run (development machine has no Docker) — build,
  healthcheck, volume, and in-container watcher behavior unverified.
- Theme sync only happens on overview route entry; search input resets after
  a perms widget save (both pre-existing, cosmetic).
- `config` hot-reload + atomic-rename editors (vim etc.) may detach the notify
  watcher — restart re-attaches.

Verification checkpoints after code changes: `cargo test` (154 tests — 44
unit + 110 integration; NaN/±Inf sort and array-sort pagination are covered
live by tests/pagination.rs `nan_sort_paginates` + `array_sort_guard` through
the server's own Mongo connection; crud_verbs.rs talks to Mongo directly
with XDB_TB_MONGO_URI, default mongodb://localhost:27017), full
auth→/q→/ls→health curl cycle, perms watcher restore cycle (see notebook
`xavierdb-build` for the exact snapshot/restore ritual). When src/ changed,
the battery needs the kill → `cargo build --tests` → restart ritual first
(§0.4).
