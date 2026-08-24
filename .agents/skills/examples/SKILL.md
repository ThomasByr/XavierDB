# Examples crate — runnable client examples

> **Script:** `examples.sh` (same dir) — `build | list | run <bin> [-- args…]`.
> Prefer it over hand-typed commands; defaults overridable via `XDB_*` env
> (see `.agents/settings/defaults.sh`).

9 examples, each = `setup_<name>.rs` (dashboard API: admin login + perms POST
creating app/token/rights) + `<name>.rs` (client API showcase). All verified
E2E against a live server; every command in examples/README.md works.

## Structure decisions

- **Standalone crate** with own Cargo.toml + lockfile (never pollutes server
  deps): `cargo run --manifest-path examples/Cargo.toml --bin <name>`.
- **ureq 2.12** with only the `json` feature — NO cookies feature: the
  dashboard session cookie is extracted from the login response `Set-Cookie`
  header manually (`split(';').next()`) and sent as a `Cookie` header on
  perms requests.
- Fully self-contained files (each duplicates ~60 lines of arg/login/call
  helpers) — copyable examples, per user choice. Hand-rolled arg parsing, no
  clap.

## Commands

```bash
cargo build --manifest-path examples/Cargo.toml
cargo run --manifest-path examples/Cargo.toml --bin setup_projection -- --admin-user <dashboard-username> --admin-pass <dashboard-password>
cargo run --manifest-path examples/Cargo.toml --bin projection
```

Setup files must pass `--admin-user` explicitly — the dashboard username is
`server.yml` admin.username (default `admin`). Re-running a setup is idempotent (it
refreshes the token hash). `set_token` ≥ 8 chars, Argon2id-hashed
server-side (~5 s per setup run).

## The 8 examples (app / db / rights)

| bin pair | app | rights |
|---|---|---|
| projection | xdb-projection | GET+POST xdb_projection — include/exclude/_id:0 + mixed → 400 INVALID_PROJECTION |
| pagination | xdb-pagination | GET+POST xdb_pagination — cursor walk limit=3 over 10 seeded docs |
| query | xdb-query | GET+POST xdb_query — $gte decimal, $regex+options, $exists, $date, $oid round-trip, raw output forms |
| indexes | xdb-indexes | GET+POST+INDEX xdb_indexes — /indexes lifecycle: 404 before collection exists, ensure 201/200 idempotent, unique index enforcing 409 on inserts, 409 same-keys-diff-options + same-name-diff-keys, TTL listing, drop 200/404/400(_id_) |
| write | xdb-write | all 5 verbs xdb_write — insert/update/PUT-404/PATCH-upsert/DELETE-404 |
| ls | xdb-ls | GET `*` — flat dbs + ?db=collections (first visible db) |
| errors | xdb-errors | GET db1 + POST/PUT/PATCH/DELETE xdb_errors — 401/403/404/409 contract |
| health | xdb-health | GET db1 (unused) — /health public + cached (2 calls, same checked_at_ms) |
| pernames | xdb-pernames | no app rules; names reader=GET, writer=GET+POST on xdb_pernames |

## Verified server facts these rely on

- **perms POST is destructive for listed fields**: `entry.allow = a.allow`
  unconditionally → omit = CLEAR. Setup files must always send the full
  allow/deny arrays (and per-name rules).
- Rule JSON: `{actions, databases, collections}` — collections omitted →
  defaults `["*"]`.
- **409 dup-key demo needs no index**: insert `data` with an explicit `_id`
  twice. Insert honors client `_id` (no stripping); `inserted_id` echoes the
  string back.
- **ObjectId seeding**: insert `{"_id":{"$oid":"<24-hex>"}, ...}` —
  idempotent (dup → 409). `_id` comes back as plain hex; `{"_id":{"$oid":hex}}`
  filters round-trip. **Hex must be exactly 24 chars** (26 chars → 400
  "bad $oid").
- Input extjson tokens: $oid/$date(rfc3339|$numberLong|ms num)/$numberLong/
  $numberInt/$numberDouble/$numberDecimal/$binary/$regex+$options/$timestamp/
  $minKey/$maxKey. Output: ObjectId→hex, DateTime→ISO,
  Decimal128→{"$numberDecimal":"…"}, NaN→{"$numberDouble":"NaN"}.
- /auth with a wrong token costs the same ~5 s (dummy-PHC timing
  equalization) — the errors showcase demonstrates this deliberately.
- **/indexes (verified 2026-08-19 in docker)**: GET needs plain GET perm
  (404 NOT_FOUND "collection does not exist" before the first insert),
  POST/DELETE need the dedicated `INDEX` action (perms ACTIONS =
  GET/POST/PUT/PATCH/DELETE/INDEX). Ensure is idempotent: 201 created /
  200 {created:false} same keys+options (auto-name "f_1_g_-1" when `name`
  omitted) / 409 same keys diff options or same name diff keys. Drop: 200
  {deleted:true}, 404 unknown name, 400 on `_id_`. TTL index demo seeds no
  dates on the indexed field so nothing ever expires. Showcase re-run safe:
  dropped indexes are re-ensured, persistent ones return 200.
- Seed idempotency trick used by all write showcases: fixed string _ids;
  duplicate 409 ignored.
- Notes for users (examples/README.md): 5 s per /auth, separate per-IP
  throttles (/auth: config 30/min; dashboard login: env 5/min), cleanup via
  `delete:true`.
