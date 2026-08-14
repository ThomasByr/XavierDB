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
   never system `python`.** On this machine the system `python` is a Microsoft
   Store alias stub; `uv` (0.11.x) provides a managed interpreter on demand
   (verified: Python 3.14 via uv). Example: `uv run --with pyyaml python -c "import yaml"`.
2. **Never `git commit`.** The user handles all commits (their GPG signing is
   misconfigured; commits fail anyway). Make changes in the working tree only.
3. **This machine has NO Docker** (not installed, not on PATH, no Docker
   Desktop — verified). Never try to run `docker` here; always run bare metal
   (§4.1). The compose/Docker setup targets other machines and is untested.
4. **On Windows the debug binary locks itself while running** — `cargo build`
   fails ("Accès refusé") until the server is killed. Restart ritual (this
   machine, Windows — each step a SEPARATE bash command):
   1. `taskkill //F //IM XavierDB.exe` → verify down (`curl /health` fails)
   2. `cargo build --tests` (rebuilds the server binary AND keeps the
      test-mode fingerprints fresh)
   3. start with
      `cd /c/Users/tbouy/code/XavierDB && ./target/debug/XavierDB.exe >> /tmp/xdb.log 2>&1 & disown`
      — the start MUST be its own bash command (not chained). A bash command
      that times out can kill the disowned server: keep commands short and
      use `--max-time` on curls.
   **Trap:** never run plain `cargo build` between `cargo build --tests` and
   `cargo test` — the normal-mode build re-invalidates the test-mode bin
   fingerprint (mongodb is built as two separate units, one per graph) and
   `cargo test` then tries to relink `target/debug/XavierDB.exe` → lock.
   The only clean sequence is: kill → `cargo build --tests` → start → `cargo test`.
   On Linux/macOS the binary is `./target/debug/XavierDB`
   (no `.exe`), rebuilds work while the server runs, and stopping it is
   `pkill XavierDB` (or `kill <pid>`).
5. Consult the notebooks (§9) for TODOs and small remarks before starting
   work; after work, update AGENTS.md (and the notebook pages) so both stay
   current. AGENTS.md must remain standalone and ready at all times.
6. `.env` is a protected path for bash on this machine (Windows) — read it via
   `read`/`cat` with care; `PASSWORD_HASH` is single-quoted in the file.
7. Credentials are machine-local: read them from `.pi/notes/credentials.md`
   (gitignored — never commit or copy them into docs/AGENTS.md). See §8.2 for
   how to obtain or regenerate them on a fresh machine.

---

## 1. What this is

A small, fast HTTP server (Rust, axum 0.8, tokio, mongodb driver) that exposes
a **MongoDB database through a REST API**: per-client authentication (JWT),
granular permissions (`authorized_keys.yml`), adaptive per-app document
limits, a binary config file with undo/redo history, and an embedded
Material-3-ish admin dashboard SPA (no JS libraries, no external fonts).
Edition 2024. No Python/Node at runtime (Node only at build time for
the dashboard TypeScript). Cross-platform Rust; developed on Windows.

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
│       └── ts/app.ts            # dashboard SPA source (~1750 lines TS) — edit here
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
| `.env` | dotenv | HOST, PORT, MONGODB_URI, MAX_WORKERS, MAX_INSERT_BATCH, TLS paths, USERNAME, PASSWORD_HASH (single-quoted!), JWT_SECRET | **No** — dotenvy reads at process start; `docker compose restart api` needed |
| `config` | XDB1 magic + crc32 + bincode | all tunables + history/redo/blocked | **Yes** — file watcher (500ms debounce) AND `/dashboard/api/config/reload` |
| `config.bak…` | same | automatic backup rotation (MAX_BACKUPS=5) on save; fallback on corruption | n/a |
| `authorized_keys.yml` | YAML | app credentials (Argon2id hashes) + layered permissions | **Yes** — file watcher (500ms debounce) + `/perms/reload` |

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

### 4.1 Bare metal (THIS machine — the only way to run here)

```bash
npm install && npm run build     # rebuild dashboard TS -> src/assets/app.js (only if TS changed)
# typecheck the dashboard TS (esbuild does NOT typecheck):
#   npx --yes -p typescript tsc --noEmit --strict --target es2020 --lib es2020,dom src/assets/ts/app.ts
cargo build                      # debug; on Windows fails while the server is running (file lock)
cargo test                       # 44 unit + 110 integration tests (tests/); needs a running server
                                 #   + MongoDB — see "Integration battery" below; tests talk to
                                 #   real Mongo unconditionally (XDB_TB_MONGO_URI, default
                                 #   mongodb://localhost:27017)
./target/debug/XavierDB          # from repo root; cwd-relative state files; no CLI args
                                 # (Windows: ./target/debug/XavierDB.exe)

# Examples (own crate, own lockfile — independent of the server build):
cargo build --manifest-path examples/Cargo.toml
cargo run --manifest-path examples/Cargo.toml --bin setup_projection -- --admin-pass <dashboard-password>
cargo run --manifest-path examples/Cargo.toml --bin projection
```

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

Windows lock gotcha: `cargo test` relinks the bin test harness (XavierDB.exe)
whenever src/ or Cargo.toml changed — kill the server first, `cargo build
--tests`, restart the server, then `cargo test` (incremental test runs don't
relink). Never run plain `cargo build` between `cargo build --tests` and
`cargo test` (it re-dirties the test-mode fingerprint — see §0.4).

- Requires a running MongoDB (default `mongodb://localhost:27017`). This
  machine: portable mongod 8.0.12 (see §8).
- Production-style: `cargo build --release` → `./target/release/XavierDB`.

### 4.2 Docker (other machines only; NEVER tested here)

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

### 4.3 Untested areas (be honest about these)

Full Docker build (aws-lc-sys in container, cmake/perl/pkg-config install),
healthcheck behavior, `${HOME}` interpolation on the user's Docker host,
notify-watcher behavior inside a container, and the dashboard UI in a real
browser (API contracts verified via curl only — see §9 gaps).

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
  merge (filter carries the `_id`); no `_id` → InsertOne. Same
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
- TS source `src/assets/ts/app.ts` (~1750 lines) → esbuild → `src/assets/app.js`
  (generated, never hand-edit). No JS libs, no external fonts.
- Full dashboard API surface (all under `/dashboard/api/*`, `xdb_admin`
  session cookie; errors same `{error, code, status}` shape): login/logout/
  session, metrics (big poll payload), block/unblock, app_weight, perms
  GET/POST(full-merge)/reload, databases, config GET/POST/undo/redo/reload/
  reset/revert/export/import, logs (structured in-memory ring, cap 3000,
  every console line incl. eprintln/panics; ?limit&before paging + app/name
  facets). See §6.
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
  `{index}`/reset/export (JSON attachment)/import.
- `GET /dashboard/api/logs` → `{lines:[{seq, raw, level, app, name}], total,
  apps, names}` (ring cap 3000; `?limit=N` (0 = all), `?before=<seq>` for
  load-older paging; `apps`/`names` = distinct identities seen in the ring,
  for the client-side filter dropdowns).
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

## 8. Environment & machine facts (this machine)

> These facts describe THIS Windows machine. The server is cross-platform:
> on Linux/macOS the binary has no `.exe`, processes are stopped with
> `kill`/`pkill` instead of `taskkill`, and `/tmp` is the real system `/tmp`.

- Windows (git-bash shell), Rust 1.97, Node v24 + npm (esbuild 0.28), uv 0.11.x
  (Python 3.14 on demand). No system python, no Docker, no mongosh.
- Portable MongoDB 8.0.12:
  `%LOCALAPPDATA%\Temp\mongodb-portable\mongodb-win32-x86_64-windows-8.0.12\bin\mongod.exe
  --dbpath C:/Users/tbouy/AppData/Local/Temp/mongodata --port 27017 --bind_ip 127.0.0.1`
  (data dir `%TEMP%\mongodata`; NOT a service; kill `taskkill //F //IM mongod.exe`).
  Test DBs: db1{items}, db2{coll_b}.
- GIT: user handles commits — never commit (§0.2).
- `.env` has NO `JWT_SECRET` on this machine → the server generates a random
  secret per start, so ALL cached JWTs die on server restart. The battery
  self-heals (probe → re-login), but remember this when debugging 401s after
  a restart. Dashboard sessions are in-memory anyway (restart = re-login).
- Test scripts/cookies kept in `%LOCALAPPDATA%\Temp` (xdb-js-example.js,
  xdb-python-example.py, xdb-cookies.txt, xdb-perms-before/grant.json,
  /tmp/xdb.log server log).

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
  entry with a freshly hashed token.
- **TLS certs** — paths are `TLS_CERT_PATH`/`TLS_KEY_PATH` in `.env`;
  regenerate with openssl (self-signed is fine for dev).
- **MongoDB** — URI in `.env` (`MONGODB_URI`, default
  `mongodb://localhost:27017`); this machine's portable mongod details in
  §8 above.

---

## 9. Notebook pages (TODOs, small remarks, history)

The pi notebook holds the fine-grained, session-by-session knowledge. Consult
it for TODOs and small remarks; promote anything durable into AGENTS.md when
it becomes load-bearing. Pages (2026-08-11):

- `xavierdb-build` — build facts, client API verification, perms test cycle, rate-computation fix history
- `xavierdb-compose` — compose/Dockerfile decisions, runtime-state mechanics, first-run behavior, caveats
- `xavierdb-local-run` — portable MongoDB setup, ops commands, machine conventions
- `xavierdb-dashboard-api` — complete dashboard API reference (exact shapes from routes_admin.rs)
- `xavierdb-dashboard-ui` — dashboard UI architecture (edit points, badge model, known gaps)
- `xavierdb-dataset-route` — /ls rename history, cursor bug fix
- `xavierdb-docs` — docs restructure, AGENTS.md, credentials layout
- `xavierdb-examples` — examples/ crate: scope decisions, verified perms/dashboard-API facts, per-example contracts (DONE 2026-08-13)
- `xavierdb-projection` — GET /q projection: design spec + implementation record (DONE 2026-08-13), verified cursor/keyset mechanics, union+strip scheme, latent dotted-sort-key bug
- `xavierdb-review` — the 3-round review campaign: per-finding verdicts, fixes, test augmentation
- `xavierdb-test-battery` — tests/ integration battery: fixture world, bootstrap, verified behaviors + spec corrections (DONE 2026-08-14)

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
  - Extraction failures (malformed or missing-field JSON bodies, malformed query strings) → **400 `{error, code:"BAD_REQUEST", status:400}`** (FIXED 2026-08-14 — previously axum's plain-text rejections, incl. 422 for missing fields, leaked through).
  - Missing/null sort values sort BEFORE NaN ascending (Mongo 8 order: null < NaN < numbers). `$gte` on a Decimal128 matches int/double values too (cross-type).
  - Watcher: a reload re-stamps the loaded bytes, so a byte-identical restore of authorized_keys.yml IS picked up automatically (FIXED 2026-08-14 — it previously required an explicit `/perms/reload`).
  - `truncated:true` + `limit_applied` = enforced cap when the client requested more than the adaptive limit; `next_cursor` only appears when the set was actually cut.
  - Insert-many (2026-08-15): `data` as array → `insert_many` (cap 1000, empty/non-object element/dup-`_id`-within-batch → 400 with NOTHING inserted; dup against existing data → 409 with ordered semantics, docs before the dup remain). Driver 3.8 facts the battery verified: `insert_many` write failures arrive as `ErrorKind::InsertMany(InsertManyError)` (NOT `WriteFailure::WriteError` — error.rs needed a dedicated arm) and `InsertManyResult.inserted_ids` is a `HashMap<usize, Bson>` (NOT BTreeMap — dbq.rs sorts by index so `inserted_ids` keeps input order). Batch size counts into per-client rate accounting (rps).
- **Dotted sort keys (`{"a.b":1}`) paginate incorrectly** — pre-existing latent bug (found 2026-08-11 during projection design; code-verified, not live-verified): `bson::Document::get` is an exact top-level lookup (no dotted resolution), so `make_next_cursor` reads a Null boundary and the array-sort guard goes blind → wrong pagination on collections sorted by nested fields. Fix = `get_path` helper + equivalence-test guard; treat as separate follow-up.
- Dashboard UI never browser-tested (API contracts verified via curl only) —
  first browser pass may reveal weight-popover overflow, legend wrap, slider feel.
- Docker setup never run (no Docker on this machine) — build, healthcheck,
  volume, and in-container watcher behavior unverified.
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
`xavierdb-build` for the exact snapshot/restore ritual). When src/ changed on
Windows, the battery needs the kill → `cargo build --tests` → restart ritual
first (§4.1).
