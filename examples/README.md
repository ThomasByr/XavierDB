# XavierDB examples

Runnable Rust client examples. Each example is a **pair of programs**:

- `setup_<name>` — uses the **dashboard API** (admin login + `POST
  /dashboard/api/perms`) to create the app id, its shared token and the
  database/collection permissions the showcase needs. Idempotent: re-running
  refreshes the token and replaces that app's rules; other apps are never
  touched.
- `<name>` — the **showcase**: authenticates as `name@app` via `POST /auth`
  and demonstrates one specific piece of the client API, printing every
  response.

Prerequisites: a running XavierDB server (see the repo README), its dashboard
password, and a MongoDB it can reach. The examples create their own
`xdb_*` databases and collections on first run — no manual setup needed.

## Quick start

```bash
# one-time: grant the app for the projection example
cargo run --manifest-path examples/Cargo.toml --bin setup_projection -- \
    --admin-user admin --admin-pass <dashboard-password>

# then run its showcase
cargo run --manifest-path examples/Cargo.toml --bin projection
```

## The examples

| setup              | showcase     | app / db                                        | demonstrates                                                                                                                                     |
| ------------------ | ------------ | ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `setup_projection` | `projection` | `xdb-projection` / `xdb_projection`             | `projection` on GET /q: include, exclude, `_id:0`, and the 400 INVALID_PROJECTION error                                                          |
| `setup_pagination` | `pagination` | `xdb-pagination` / `xdb_pagination`             | keyset cursor walk: follow `next_cursor` with `limit=3` until `has_more:false`                                                                   |
| `setup_query`      | `query`      | `xdb-query` / `xdb_query`                       | filter operators + extended JSON: `$gte`, `$regex`+`$options`, `$exists`, `$date`, `$oid`, and the `$numberDecimal`/`$numberDouble` output forms |
| `setup_write`      | `write`      | `xdb-write` / `xdb_write`                       | the write verbs: insert, update (auto-`$set`), PUT (404 on no match), PATCH upsert (201/200), DELETE (404 on no match)                           |
| `setup_ls`         | `ls`         | `xdb-ls` / `*` (GET)                            | GET /ls: flat database list, then `?db=X` collections                                                                                            |
| `setup_errors`     | `errors`     | `xdb-errors` / db1 (GET), `xdb_errors` (writes) | the error contract `{error, code, status}`: 401 bad token, 403 denied action, 404 no match, 409 duplicate `_id`                                  |
| `setup_health`     | `health`     | `xdb-health` / db1 (unused)                     | GET /health is public and cached: two quick calls return the same document                                                                       |
| `setup_pernames`   | `pernames`   | `xdb-pernames` / `xdb_pernames`                 | name-level permissions: `reader` (GET) vs `writer` (GET+POST) under one app token                                                                |

## Shared options

Every program accepts these flags (and environment variables where noted):

| flag           | env              | default                 | meaning                                                                    |
| -------------- | ---------------- | ----------------------- | -------------------------------------------------------------------------- |
| `--base-url`   | —                | `http://127.0.0.1:8000` | server base URL                                                            |
| `--admin-user` | `XDB_ADMIN_USER` | `admin`                 | dashboard username (setup only)                                            |
| `--admin-pass` | `XDB_ADMIN_PASS` | — (required)            | dashboard password (setup only)                                            |
| `--token`      | `XDB_TOKEN`      | `demo-token-change-me`  | the app's shared secret (setup + showcase must agree)                      |
| `--app`        | —                | per example             | app id (showcase only; `pernames` also uses fixed names `reader`/`writer`) |
| `--name`       | —                | `demo`                  | name id, the identifier is `name@app` (most showcases)                     |

The dashboard password and the app tokens are never stored in files — pass
them per run. The default token exists so the commands above work out of the
box against a local dev server; use `--token` (or `XDB_TOKEN`) for anything
else.

## Notes

- **Each `/auth` takes ~5 seconds**: Argon2id (64 MiB, t=3, p=4) is verified
  for every login by design, to make brute force expensive. Showcases log in
  once or twice; expect a short wait.
- **Login throttle**: `/auth` and the dashboard login share a per-IP limit
  (default 30/min). Do not run many examples back to back inside one minute.
- **Seeding is idempotent**: showcases insert their demo documents with
  fixed `_id`s, so re-running them is safe (the duplicate-key 409 is
  expected and ignored).
- **No API creates databases** — MongoDB creates a collection on the first
  insert via `/q`; the examples rely on that.
- **Cleanup**: remove an app with a perms POST carrying `delete:true` (or
  delete it in the dashboard Clients view). The `xdb_*` databases can be
  dropped with any MongoDB tool.
- The setup files rewrite `authorized_keys.yml` via the dashboard API, which
  re-formats the file and drops comments (known server behaviour).
