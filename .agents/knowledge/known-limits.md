# Known limits & open items

## Known limits (by design, not bugs)

- Admin sessions in-memory → restart = re-login.
- Dashboard rewrite of authorized_keys.yml loses its comments.
- Keyset pagination refuses (400) to continue past a page whose sort field
  contains an **array** value — MongoDB's element-wise array sort cannot be
  represented in a keyset cursor; silent loss/loops would be worse. NaN/±Inf
  sort values ARE handled (NaN sorts first ascending on MongoDB 8).
- `/auth` and dashboard login have SEPARATE per-IP throttles (dashboard
  login: server.yml `admin.max_logins_per_ip_per_minute`, default 5; `/auth`:
  `config.auth.max_per_minute_per_ip`, default 30). Both key on the peer
  socket IP — `X-Forwarded-For` is deliberately NOT trusted (no proxy in the
  deployment; the header is client-controlled). Window is a fixed wall-clock
  minute: up to 2× the limit can pass across a minute boundary.
- `/auth` + dashboard login: Argon2id verify runs on the tokio blocking pool
  (never on async workers); unknown apps/usernames verify against a fixed
  dummy PHC so response timing doesn't reveal whether an identity exists;
  blocked ids are checked before the hash (403 regardless of token). All
  auth failures return the identical `UNAUTHORIZED` body.

## Known gaps / things to check

- **GET projection: IMPLEMENTED (2026-08-13)** — `projection` param (JSON
  object, INVALID_PROJECTION 400), union+strip scheme keeps the keyset cursor
  correct (Mongo always sees sort fields + `_id`; client sees only requested
  fields). Dotted/nested projection keys and `$`-operators rejected
  (top-level only, v1). Details: architecture.md Projection.
- **Verified live by the battery (2026-08-14)** — behaviors worth knowing:
  - Include-only projections STRIP `_id` unless explicitly requested:
    `{name:1}` → docs have only `name`; `{name:1,_id:1}` keeps it. `{_id:0}`
    alone returns everything except `_id` (FIXED 2026-08-14 — it previously
    collapsed to `{}`).
  - Dots are valid in COLLECTION names (`bad..name` OK) — 400 only for dots
    in the db segment. MongoDB 8.0.12 also ACCEPTS `$`-prefixed field names
    in stored documents (they round-trip literally).
  - Extraction failures (malformed or missing-field JSON bodies, malformed
    query strings) → **400 `{error, code:"BAD_REQUEST", status:400}`** (FIXED
    2026-08-14 — previously axum's plain-text rejections, incl. 422 for
    missing fields, leaked through). `filter=%zz` decodes leniently →
    INVALID_FILTER (not a query rejection); `limit=abc` → BAD_REQUEST via the
    custom extractor.
  - Missing/null sort values sort BEFORE NaN ascending (Mongo 8 order:
    null < NaN < numbers). `$gte` on a Decimal128 matches int/double values
    too (cross-type).
  - Watcher: a reload re-stamps the loaded bytes, so a byte-identical restore
    of authorized_keys.yml IS picked up automatically (FIXED 2026-08-14 — it
    previously required an explicit `/perms/reload`).
  - `truncated:true` + `limit_applied` = enforced cap when the client
    requested more than the adaptive limit; `next_cursor` only appears when
    the set was actually cut.
  - Insert-many (2026-08-15): `data` as array → `insert_many` (cap 1000,
    empty/non-object element/dup-`_id`-within-batch → 400 with NOTHING
    inserted; dup against existing data → 409 with ordered semantics, docs
    before the dup remain). Driver 3.8 facts: see architecture.md Batch
    write driver facts. Batch size counts into per-client rate accounting.
- **Array `_id` (`{"_id":[]}`) maps to 500** — Mongo error code 53 ("_id"
  cannot be an array) is not in the client-code list in error.rs; arguably a
  client error → 400. Pre-existing for single writes, consistent for the
  insert/upsert batch arms (flagged 2026-08-15, not changed). Fix = add 53 to
  the client-codes list in error.rs (1-line).
- **Dotted sort keys (`{"a.b":1}`) paginate incorrectly** — pre-existing
  latent bug (found 2026-08-11 during projection design; code-verified, not
  live-verified): `bson::Document::get` is an exact top-level lookup (no
  dotted resolution), so `make_next_cursor` reads a Null boundary and the
  array-sort guard goes blind → wrong pagination on collections sorted by
  nested fields. Fix = `get_path` helper + equivalence-test guard; treat as
  separate follow-up.
- Dashboard UI not yet browser-tested (API contracts verified via curl and
  jsdom repros) — a first browser pass may reveal weight-popover overflow at
  the viewport bottom, legend wrap, slider feel. Other pre-existing cosmetic
  items: theme sync only on overview route entry; search input resets after a
  perms widget save. Declined/not implemented: beforeunload dirty guard.
- **Docker (verified 2026-08-16, Docker Desktop 29.7.2/WSL2):** full build +
  compose up + healthchecks work; battery vs the docker API is 108/110. The
  app's perms/config/TLS hot reloads are pure inotify and inotify events do
  NOT flow over Docker Desktop bind mounts (VirtioFS; even container-side
  writes never fire — virtiofsd has no FUSE notify) → `watcher_reload` is
  expected to FAIL when the API runs on Docker Desktop (bare metal / Linux
  hosts pass). Settings precedence: env var > server.yml > default (so
  compose injects HOST/MONGODB_URI into the container; bare metal uses the
  file) — except admin.username/password_hash, which always come from the
  file (Windows always sets USERNAME). Details: skills/docker.md.
- `config` hot-reload + atomic-rename editors (vim etc.) may detach the notify
  watcher — restart re-attaches.

## Deferred work

- **Docker build speed — pending (2026-08-17).** Two safe fixes identified
  and measured (context exclusions in .dockerignore; copy Cargo.lock into the
  dummy-main layer) plus one dangerous pattern to avoid (target-dir cache
  mount leaks the dummy binary). Reverted, not applied — details:
  skills/docker.md "Build speed" section.
- **Performance verification — deferred (2026-08-11, user decision).** No
  perf work in the 3 review rounds (correctness/security/contracts only).
  No benchmarks, no profiling, no load tests in the repo. Natural first steps
  when picked up: criterion microbenchmarks for json_to_bson/bson_to_json;
  wrk/hey load test against /q/ (p50/qps via /health, watch adaptive-limit
  behavior under pressure); memory-growth check on the two accepted leaks
  (stats DashMaps never evicted, cursor registry count-only eviction).
- Covered queries: projections that include the sort fields are the
  covered-query shape — verify with explain() during the perf pass.

## Verification checkpoints after code changes

`cargo test` (160 tests — 50 unit + 110 integration; NaN/±Inf sort and
array-sort pagination are covered live by tests/pagination.rs
`nan_sort_paginates` + `array_sort_guard` through the server's own Mongo
connection; crud_verbs.rs talks to Mongo directly with XDB_TB_MONGO_URI,
default mongodb://localhost:27017), full auth→/q→/ls→health curl cycle, perms
watcher restore cycle (skills/perms-watcher-ritual.md). When src/ changed,
the battery needs the kill → `cargo build --tests` → restart ritual first
(skills/restart-ritual.md).
