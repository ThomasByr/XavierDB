# Architecture

## 1. Auth

- `POST /auth` `{identifier, token}` → validates identifier against
  `authorized_keys.yml` + Argon2id-verifies the shared token → returns
  `{token, token_type:"Bearer", expires_in:5400, identifier}` + `Set-Cookie:
  xdb_token` (HttpOnly; Secure under TLS). 401 bad creds, 403 BLOCKED, 429
  throttle.
- JWT: HS256, secret = server.yml `auth.jwt_secret` or random-per-start; lifetime from
  `config.global.jwt_token_lifetime_minutes` (default 90). Expired/malformed →
  401 with a 5s leeway; reason swallowed. Client loop: on 401 re-auth, on 403
  do NOT re-auth.
- Blocked ids (in `config.blocked`) → 403 BLOCKED at `/auth`.
- **The app token is shared by every name under an app** (one Argon2id hash
  per app in authorized_keys.yml): any holder can /auth as ANY `name@app` —
  existing or not (new names are auto-added to the yml on first login). The
  name_id is a permission-routing label, not a credential; name-level rules
  separate identities within the app only. Each name needs its own /auth for
  its own JWT (sub = exact name).
- Dashboard sessions: in-memory DashMap (`xdb_admin` cookie, Path=/dashboard,
  TTL `config.auth.session_ttl_hours` default 24) — **restart = re-login**.
- Login throttles: `/auth` and dashboard login have SEPARATE per-IP 1-minute
  windows. `/auth` always uses `config.auth.max_per_minute_per_ip` (default
  30, dashboard-editable); dashboard login uses server.yml
  `admin.max_logins_per_ip_per_minute` (default 5, clamped 1..=10_000).
  **Client IP source (2026-08-18):** socket peer by default; when server.yml
  `network.trust_proxy_headers` (env `TRUST_PROXY_HEADERS`, compose sets it
  true) is on, the proxy header wins — `X-Real-IP`, else the LAST
  `X-Forwarded-For` entry (the proxy-appended one; must parse as an IP or is
  ignored). Helpers `routes_q::{proxy_ip, effective_ip, effective_addr}`
  (unit-tested). Safe in the compose deployment because the port is
  published to 127.0.0.1 only (nginx is the sole connector, sets X-Real-IP).

### Auth Q&A (verified from code — auth.rs, perms.rs)

- **Why JSON↔BSON conversions at all?** The API is a REST/JSON facade over
  BSON-native MongoDB. `json_to_bson` (requests): extended-JSON tokens decoded
  (`$oid`, `$date`, `$numberLong/Int/Double/Decimal`, `$binary`, `$regex`+
  `$options`, `$timestamp`, `$minKey/$maxKey`); u64 > i64::MAX becomes
  Decimal128; `$where`/`$function` rejected (400). `bson_to_json` (responses):
  type-fidelity rules so re-inserting a response never silently changes types
  (ObjectId → hex, DateTime → ISO, NaN/±Inf → `{"$numberDouble":…}`,
  Decimal128 → `{"$numberDecimal":…}`); cursor page values additionally use
  canonical extended JSON. Without the fidelity rules, NaN read back as null
  and Decimal128 as a plain string (both were real bugs).
- **Can an authenticated client act as any name_id under its app?** The JWT
  is bound to the exact `sub` issued at /auth and signed with the server
  secret — name1's JWT cannot be re-claimed as name2, and per-request
  authorization uses the JWT's sub+app claims. BUT the app token is shared by
  every name under the app: anyone holding it can /auth as ANY `name@app`.
  Name rules separate identities *within* the app only — not a security
  boundary against token holders.
- **Does each name_id need its own /auth call?** Yes. A JWT is per-name;
  name-level rules apply per JWT. Token expiry is per-user.
- Verified mechanics: identifier = `name@app`, each part 1–64 chars of
  `[A-Za-z0-9-_.:~]`. /auth: throttle (per peer IP) → parse → check_block
  (403) → spawn_blocking Argon2id verify (dummy PHC for unknown apps, timing
  equalized) → auto-add name → sign JWT. Claims: sub, app, iat, exp, jti;
  5 s leeway. Blocked: `name@app` exact or bare `app` → 403 BLOCKED.

## 2. Permissions (`authorized_keys.yml`)

- Structure: `apps: {app_id: {token_hash, allow: [rules], deny: [rules], names:
  {name: {allow, deny}}}}`. Rule = `{actions, databases, collections}`.
  Globs `*` and `?`. Actions = HTTP verbs `GET/POST/PUT/PATCH/DELETE` **plus
  `INDEX`** (manage indexes on /q/{db}/{coll}/indexes — default-deny, a
  schema-level capability separate from document writes). Template:
  `authorized_keys.yml.example`.
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

## 3. `/q/<db>/<coll>` proxy (routes_q.rs)

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
  `state.max_insert_batch` — server.yml `runtime.max_insert_batch`, default
  1000 (main.rs, routes_q.rs `MAX_INSERT_BATCH`), must be ≥ 1, also published
  top-level in
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
  branch — NaN sorts first ascending on MongoDB 8; a `$gt` there would miss
  the -Inf tie-group, caught by the equivalence test). ARRAY sort values are
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

### Index endpoints (GET/POST/DELETE /q/{db}/{coll}/indexes)

- Perm model: GET lists indexes under the plain `GET` action (read access
  ⇒ seeing index names/keys); POST (ensure) and DELETE (drop) both require
  the dedicated `INDEX` action — one capability for index management, not
  split by verb (document-DELETE must not imply dropping a unique index,
  and INDEX must not imply document-delete).
- ensure = idempotent createIndex, decision table: no index on those keys →
  create → 201 `{created:true,name}` (name auto-generated when omitted,
  `field_1_dir_…` style); same key pattern (any name) + same options →
  200 `{created:false,name:existing}`; same name different keys → 409; same
  keys different options (unique/sparse/TTL/partial filter) → 409 (changing
  a TTL would need collMod — refused loudly, v1). Implemented by listing
  first, then create_index — the driver's create gives no created/existed
  info.
- Flat request body `{keys, name?, unique?, sparse?, expire_after_seconds?,
  partial_filter_expression?}`; keys validated server-side (each value 1/-1
  or an index-type string like "text"/"2dsphere"/"hashed" → 400 otherwise).
  `$where`/`$function` rejected in partial_filter_expression. Drop is
  by NAME only (what GET returns); `_id_` refused with 400, unknown name
  404 (pre-checked — the raw driver error would be a 500).
- listIndexes on a missing collection fails with Mongo code 26 → mapped to
  a clean 404. createIndexes client errors (codes 67/85/86/118) map to 409
  with the sanitized server message.
- Driver facts (mongodb 3.8): `mongodb::IndexModel` +
  `mongodb::options::IndexOptions` (the `mongodb::index` module is PRIVATE —
  only IndexModel is re-exported at the root; IndexOptions lives under
  `mongodb::options`); `expire_after` is `Option<Duration>` (seconds ×),
  `IndexOptions::default()` exists (non_exhaustive but Default);
  `list_indexes` streams `IndexModel` (options field is `Option<IndexOptions>`,
  name lives INSIDE options); `create_index` → `CreateIndexResult{index_name}`.

### Projection implementation map (v1, 2026-08-13)

- error.rs: `InvalidProjection` variant (code "INVALID_PROJECTION", 400).
- dbq.rs: `ProjectionStyle`/`Projection`, `parse_projection(&Value)`,
  `projection_document(...)`, `projection_strip_fields(...)`,
  `bson_to_json_projected(&Document, &BTreeSet)`; `find_page` takes
  `projection: Option<Document>`.
- routes_q.rs: `FindParams.projection: Option<String>`, `parse_projection_json`;
  output mapped through `bson_to_json_projected`.
- `{_id:0}` alone → Exclude style → no projection sent → all fields except
  `_id` (real Mongo semantics; previously collapsed to `{}` docs — FIXED
  2026-08-14). `{_id:1}` alone unchanged. Include-only projections strip `_id`
  unless explicitly requested (`{name:1}` → only `name`; `{name:1,_id:1}`
  keeps it) — v1 contract.
- Regression net: env-gated `keyset_pagination_equivalence` unit test, phase 3
  covers projection (coll3 × 2 sorts × 2 limits × none/include {b}/exclude
  {c,_id:0}) — `_id` walk must equal the full-scan baseline AND visible fields
  == source ∩ expected exactly. It caught a wrong `$gt`→`$gte` mistake
  mid-implementation.

### Batch write driver facts (mongodb 3.8 — hard-won, battery-verified)

- `insert_many` write failures arrive as `ErrorKind::InsertMany(InsertManyError)`
  — NOT `ErrorKind::Write(WriteFailure::WriteError)`; error.rs needs a
  dedicated arm. `InsertManyResult.inserted_ids` is a `HashMap<usize, Bson>`
  (NOT BTreeMap) — dbq.rs sorts by index so `inserted_ids` keeps input order.
- `Client::bulk_write(models)` — `.verbose_results()` takes NO bool argument
  (type-level marker returning `BulkWrite<VerboseBulkWriteResult>`);
  `VerboseBulkWriteResult.insert_results: HashMap<usize, InsertOneResult>`
  (map `.inserted_id` yourself); `update_results` →
  `UpdateResult.upserted_id: Option<Bson>` (filter_map for upserted ids).
  Models: `mongodb::options::{WriteModel, UpdateOneModel, InsertOneModel}`,
  `mongodb::Namespace::new(db, coll)`; `.update()` takes
  `impl Into<UpdateModifications>` (From<Document> exists).
  `doc.remove("_id")` strips the id for the $set payload; the filter carries
  it and Mongo re-adds it on upsert-insert.
- Client error codes → 400: 2|9|14|121|17287|51075|51091|31034; any 11000 →
  409 CONFLICT; else 500.
- Validator collections via driver: `db.collection::<Document>(c).drop().await`
  (idempotent on missing) then `db.create_collection(c).validator(doc).await`.
- `count_documents(filter: Document)` (action API, returns u64).

## 4. `/ls` (replaced `/q/dataset` — no alias, that route now 404s; `dataset` is NOT reserved)

- `GET /ls` → `{databases: ["a","b"], next_cursor, has_more, limit_applied}` —
  FLAT name strings, permission-filtered, cursor-paginated over dbs only.
- `GET /ls?db=X` → `{db:"X", collections:[...]}`; 404 when X doesn't exist
  (checked vs `dbq::list_databases`); 403 when X exists but the caller has no
  access (`perms.listable_databases(...)` empty → 403). 401 only from
  `authenticate()`.
- Handler: `routes_q::list_visible`, registered on the top-level router.
- Listing cursor: `sort: [("name",1)]` ONLY — pagination is pure name-based
  `retain(|d| d > last)`; a previous build emitted sort with 2 entries but
  `last` with 1, and `Cursor::decode` requires `last.len() == sort.len()` →
  every second page failed "wrong listing cursor". Don't add a second sort
  column without a matching boundary value.

## 5. Adaptive limit (metrics.rs)

- Per-app document limit, re-derived every tick (default 5s):
  `lat_err = max(0,(p50−target)/target)`, `pressure = max(0,(cpu−60)/40,
  (mem−70)/30)`, `shrink = 1/(1+K_l·lat_err+K_p·pressure)`; internal limit ×=
  shrink if <1 else × growth_rate, clamped [min_limit, max_limit]; enforced =
  `round(internal · multiplier · weight).clamp(min,max)`. Internal STARTS at
  max_limit on first tick. Per-app `weight` in `config.rate_limit.weights`
  (0.1–10, dashboard-editable, default 1.0). Higher weight = bigger share of
  the page limit under load (never above max_limit).
- Rates are delta-based: `ClientStats.last_total` cumulative counters, EMA
  smoothing (alpha = `config.rate_limit.ema_alpha`), decay to 0 when idle;
  history = 120 samples per tick. Both app AND name keys get rates/sparklines;
  adaptive limit is app-only (`key[4..]` strips the `app:` prefix).
- Requests over the limit: first page + `next_cursor` (client must paginate;
  the server never loads a huge set into RAM).

## 6. Config file (config.rs)

- `XDB1` magic + crc32 + len + bincode; unknown version refused → backup fallback.
  Atomic writes (tmp + fsync + rename); backups `config.bak`, `config.bak.2`,
  … rotate (MAX_BACKUPS=5, chain is real: oldest dropped, rest shifted, fresh
  copy — verified by test). History capped at 10k snapshots `{ts, desc, path,
  snapshot, by}`; **snapshots are FLAT (no history/redo inside)** — nesting
  them would double the file size on every mutation; undo/redo/revert rebuild
  the entry list from metadata. API returns history NEWEST-first and
  `revert {index}` takes the NEWEST-FIRST display position (0 = newest).
- Sanitization: `ConfigFile::sanitize()` (config.rs) is the single source,
  applied on save/import/revert AND `load_from_disk` (a corrupted config with
  an OOM-able max_limit or an overflowing jwt lifetime/session ttl can
  otherwise be loaded). Exact clamps: min_limit ≥ 1 and ≤ 10 000;
  max_limit ∈ [min_limit, 10 000] (min > max would panic the metrics loop —
  max is raised to min); multiplier ∈ [0.05, 20]; target_latency ∈ [1, 60 000];
  growth ∈ [1, 2]; tick ∈ [1, 3600]; ema ∈ [0.01, 0.9]; sensitivities ∈
  [0, 20]; health ttl ∈ [1, 3600]; dashboard poll ∈ [0.1, 3600] (f64);
  smoothing ∈ [1, 60]; log_level ∈ {info, debug}; theme ∈ {system, light,
  dark}; per-ip ∈ [1, 10 000]; session ttl ∈ [1, 720]; jwt ∈ [1, 43 200].
- Key fields (defaults): global{jwt_token_lifetime_minutes=90,
  permission_file="authorized_keys.yml"}, rate_limit{min=1, max=200,
  multiplier=1.0, target=50, pressure_sens=1.5, latency_sens=1.0, growth=1.15,
  tick=5, ema=0.2, weights{}}, health{ttl=5}, dashboard{poll=2, smoothing=5,
  log_level="info", theme="system"}, auth{per_ip=30, session_ttl_h=24}, blocked[], history[],
  redo[].
- `dashboard.poll_seconds` is u64 → f64 (2026-08-14); bincode uses VARINT int
  encoding → legacy config files fail to decode → defaults (accepted, no
  migration).

## 7. Health

- `GET /health` (public, cached, default TTL 5s):
  `{status:"ok|degraded|unhealthy", checked_at_ms, next_refresh_seconds,
  compute_latency_ms, qps, max_insert_batch, constants:{max_insert_batch,
  jwt_token_lifetime_seconds, max_document_limit}, app:{status, uptime_s, p50_latency_ms,
  total_requests, active_cursors}, mongodb:{reachable, ping_latency_ms,
  error}}` — 200 only when ok, else 503. `max_insert_batch` is the
  insert-batch cap (server.yml runtime.max_insert_batch), static per process — the battery
  reads it from here so cap-boundary tests work with custom values.
  `constants.jwt_token_lifetime_seconds` mirrors the effective
  config.global.jwt_token_lifetime_minutes × 60 — auth_flow::login_ok
  asserts expires_in against it (the lifetime is dashboard-editable, so the
  test must not hardcode the default).
  `constants.max_document_limit` = config.rate_limit.max_limit — the ceiling
  the enforced per-app limit never exceeds (enforced = clamp(round(internal
  × multiplier × weight), min, max)); limit-bound integration assertions
  must read it instead of assuming 200.
- Verified live: mongod kill → `unhealthy`/`reachable:false`/HTTP 503 (the
  supervised health loop keeps refreshing — no stale "ok"); mongod restart →
  auto-recovery to ok/200 without server restart.

## 8. TLS (tls.rs)

- Optional TLS; BOTH cert and key are hot-reloaded. Verified live:
  matched-pair rotation reloads without restart (new CN served); key-file
  mismatch → warn + keep old; garbage cert → "no certificate found"
  fail-safe, listener unaffected by bad reloads.

## 9. Dashboard

- Embedded SPA (`include_str!` at compile time, served no-cache under
  `/dashboard/`), hash-routed, 4 pages: `#/overview | #/clients | #/config |
  #/logs`. Permissions/rate-limit pages were removed (2026-08 rework).
- TS source `src/assets/ts/app.ts` (~2050 lines) → esbuild → `src/assets/app.js`
  (generated, never hand-edit). No JS libs, no external fonts.
- Full dashboard API surface (all under `/dashboard/api/*`, `xdb_admin`
  session cookie; errors same `{error, code, status}` shape): login/logout/
  session, metrics (big poll payload), block/unblock, app_weight, perms
  GET/POST(full-merge)/reload, databases, config GET/POST/undo/redo/reload/
  reset/revert/export/import, logs (rotating FILES on disk, server.yml-configured
  log.files/log.size_mb — no in-memory ring; ?limit&before paging + app/name
  facets; every console line incl. eprintln/panics). Contracts: see api.md.
- Config tab: EXPLICIT save — slider edits alone don't persist (a page
  reload discards them); an amber "unsaved changes" dirty pill is pinned to
  the card title line (`margin-left:auto` inside the flex `h3` — never in the
  buttons row, where a full wrapping line leaves no free space and the pill
  lands inline between Save/Undo), Save is disabled while clean, and an
  in-flight `configSaving` guard prevents double POSTs.
- Logs box colors are theme-aware `--logs-*` tokens, defined in ALL THREE
  theme blocks (`:root` light, `prefers-color-scheme: dark`, forced
  `[data-theme="dark"]`) — any new theme-aware token must land in all three.
- Browser-behavior debugging without a browser: a jsdom repro drives the
  SERVED bundle (fetch `/dashboard/` index.html + app.js — re-fetch after
  EVERY rebuild, the embed is compile-time), stubs fetch/matchMedia, and
  simulates clicks. Pattern: see skills/dashboard-rebuild.md.

### Dashboard UI architecture (src/assets/ts/app.ts)

- Topbar: `.mongo-widget` = pill containing `#mongo-btn` (`.mongo-status`:
  `#mongo-dot` + "MongoDB status" text) and `#mongo-refresh` (↻ INSIDE the
  pill). Dot maps /health status ok → `.ok` green, degraded → `.warn`
  orange, unhealthy → `.bad` red via `updateMongoStatus(h)` (called from
  BOTH `renderOverviewData` and `renderClientsData`); tooltip carries ping
  latency / error. `refreshMongoStatus()` fetches `/health` directly (public
  root route, NOT via `api()` which prefixes `/dashboard/api`), updates
  `lastMetrics.health` + the dot, returns the doc. `#mongo-refresh` = silent
  refresh; `#mongo-btn` click = same + snackbar with fresh status/ping. The
  old standalone `#refresh-btn` (metrics poll) is gone.

- Overview: blocked-apps alert strip (`.ov-alert`, hidden while no app is
  blocked; `renderOvAlert` lists the blocked apps as `.badge.bad`), 4 stat
  chips + 5 mini chart cards (CPU, Memory, Disk, Download, Upload) via
  `drawMini()`, plus an "App traffic" card (`renderOvTraffic`): the top 6
  apps by RPS (`OV_TOP_APPS`), rebuilt every poll like the limits table —
  columns weight / trend (70×22 sparkline from `rps_history`, drawn only
  AFTER the row is attached — `clientWidth` is 0 before) / rps / p50 / limit
  / status badge; header summary = active count, summed rps, worst p50,
  lifetime `health.app.total_requests`.
- Clients: `renderClients()` builds the shell once; `renderClientsData(m)`
  per poll does in-place `[data-role=...]` updates and rebuilds only the
  limits + cursors tables. `mergedApps(m)` = live + file-only apps. Perms
  drift detection: `m.config.perms_version !== permsData.version` →
  `loadPermsData()`. Expansion via a `clientsExpanded` Set; detached scopes
  (`detachApps`/`detachNames`) persist only when they carry content.
  Weight chip → `openWeightPop` popover (0.1–10 step 0.1, auto-POST
  /app_weight on release); `w-alt` accent when ≠ 1.
- Permission editor badge model: 6 badges per row (5 verbs + INDEX), click cycles allow → deny
  → inherit (explicit SOLID / inherited DASHED / none HOLLOW; collections
  inherit-db GRAY FILL). Collections: caret expands a db row → real
  collections + overrides + "+ add". Globs: own badges + ↺ + ✕; ACTIVE globs
  lock matching rows + 🔒. Save: `queuePermsSave()` chain → POST /perms →
  GET /perms → `rebuildOpenPanels()`. Search clears after save (pre-existing).
- Config tab form: field spec `groups: [name, hint|null, CfgField[]][]`;
  `CfgField{path,label,kind,min?,max?,step?,unit?,prefix?,options?}`; kinds
  range/text/select. 3 groups (Health merged into Dashboard;
  `.config-grid` auto-fit): General (permission_file text, JWT lifetime,
  per-IP auth, session TTL), Rate limiting (multiplier, target p50,
  sensitivities, growth, min/max docs, tick, ema), Dashboard (poll,
  smoothing, health TTL, log_level, theme). Save flow: `#cfg-save` onclick →
  `structuredClone(configData.config)` → collect
  `form.querySelectorAll("input[data-path], select[data-path]")` →
  POST `/config` → re-render + snack. `renderConfigForm` rebuilds ALL sliders
  from `configData.config` on every load/save; dirty state via module-level
  `configDirty` + `markCfgDirty()`; failed save keeps dirty + re-enables
  Save. Dragging a slider back to its original value still counts as dirty
  (no baseline diff) — accepted. Blocked identifiers card = full-width below
  the columns.
- Logs tab: file-backed store (see config-world.md + api.md logs endpoint).
  `.logs-head` h3 + [↻ Refresh] [⬇ Download] top-right; `.logs-filterbar`
  (position:relative) = "Add filter" + `#logs-fbadges` + retention line +
  `.logs-pop` popover NESTED INSIDE the filterbar (must stay a CHILD of the
  positioned anchor; the outside-close handler must use the CLASS selector —
  it once used an ID selector on a class-only element and every mousedown
  closed the popover). `.logs-box` flex-fills the card
  (`calc(100vh - 64px - 48px - 14px)`). Multi-value filters:
  `logFilters{levels[], loggers[], apps[], names[], regex}` — OR within a
  category, AND across; badges per category (`.f-group` + `.f-chip`);
  popover add-flow with typeahead + suggestion chips; name picker from
  (app,name) facets. Paging: `LOG_PAGE = 300`; scroll near top (40px) →
  `?limit=300&before=<oldest loaded seq>`; `logNoMore` stop. Download =
  `/logs` no params → raws → blob.
- Design system (styles.css): tokens in three blocks — `:root` (light),
  `@media (prefers-color-scheme: dark) :root:not([data-theme="light"])`,
  `:root[data-theme="dark"]` (forced). Primary #6d4aff. `.config-grid` =
  `repeat(auto-fit, minmax(300px, 1fr))`. Badges: `.badge` pill +
  `.ok`/`.bad`/`.warn`/`.info`/`.primary` variants; `.hidden {
  display:none !important }`. `.card h3 { display:flex; align-items:center;
  gap:8px }` — right-edge pinning via `margin-left:auto` on a child works
  there reliably (see dirty-pill placement above).
- Per-request DEBUG log lines only when `dashboard.log_level = "debug"`
  (hot-reloadable via `reload::Layer` + `with_filter`; hook applied at config
  save/reload/watcher). The battery runs at info → ~0 DEBUG lines by design.

### Request log line formats (2026-08-18: peer addr added)

Every identity-carrying log line ends with the effective client address
  (`from IP` or `from IP:PORT`):
  - `network.trust_proxy_headers` ON (compose/prod): the proxy header IP
    (`X-Real-IP`, else last XFF entry) — no port, the proxy doesn't forward
    one; verified live 2026-08-18.
  - OFF (bare metal default): the socket peer `IP:PORT`. Behind a Docker
    port-forward that's the bridge gateway (172.x.0.1), not the real client
    IP. Behind compose+nginx in prod, the flag is ON via the
    `TRUST_PROXY_HEADERS=true` env in compose.yaml (safe: port published to
    127.0.0.1 only).
- `/q` + `/ls` debug trace (routes_q): `GET /q/db/coll from 127.0.0.1:55555 as name@app`
  — identity stays LAST so `log_identify`'s `" as "` split keeps working.
- `/auth` login lines (routes_misc, INFO/WARN): `login OK: name@app from 127.0.0.1:55555`
  (also failed/blocked; `login throttled: IP:PORT`).
- `state.rs log_identify` strips the trailing `" from <addr>"` on login lines
  before extracting the app facet — legacy lines without an addr still parse
  (unit test `log_identify_formats`).
