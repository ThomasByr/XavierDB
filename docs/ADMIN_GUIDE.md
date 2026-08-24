# Admin guide

The dashboard lives at `/dashboard/` (or `https://…` when TLS is enabled).
It is a single static page: no libraries, no build step at runtime — the
TypeScript is compiled at build time and embedded in the binary. Four
hash-routed views: `#/overview`, `#/clients`, `#/config`, `#/logs`.

## First login

- Username: `admin.username` from `server.yml` (default `admin`).
- Password: if `admin.password_hash` is empty, the server **generates a strong
  64-character password on startup, prints it once in the terminal** and
  stores only its Argon2id hash in `server.yml`. Change it by editing
  `server.yml` (`$` needs no quoting in YAML) and restarting.

Sessions are in-memory and last `auth.session_ttl_hours` (default 24 h);
a restart logs everyone out. Login attempts are throttled per IP by
`admin.max_logins_per_ip_per_minute` from `server.yml` (default 5/min,
clamped 1–10 000, restart to apply) — a **separate counter** from the client
`/auth` throttle, which uses `auth.max_per_minute_per_ip` in the binary
`config` (default 30/min, dashboard-editable). The client IP is the socket
peer address, or `X-Real-IP` / the last `X-Forwarded-For` entry when
`network.trust_proxy_headers: true` in `server.yml` (default `false`).

## Pages

### Overview
System metrics polled every few seconds (per-browser poll interval, default
2 s — see the Settings popover in the top bar): CPU, memory, disk, network
(KB/s), plus QPS, MongoDB ping and uptime. Graphs are smoothed client-side
(TensorBoard-style EMA; the coefficient also lives in the Settings popover,
0 = raw — both are saved in localStorage, i.e. per browser). The golden rule
for the Config page: *anything that could need tuning is tunable here.*

### Clients
Tree of `app_id → name_id`, each node showing smoothed requests/second
(number + sparkline) and p50 latency, plus a live **adaptive-limits table**
with the full breakdown (`lat_err`, `pressure`, `shrink`, internal vs
enforced limit).

Per app row:

- **Weight chip** (`×1.0` by default) — a slider popover sets the app's
  adaptive-limit weight (0.1–10, step 0.1). Enforced limit =
  `limit × multiplier × weight`.
- **Block/unblock** — blocking an app blocks every name under it; blocking
  a name blocks only that one. Blocked requests get `403` with code
  `BLOCKED` immediately (no token expiry needed). Blocks persist in the
  config file.

The **Live cursors** card lists active pagination cursors (collection, age,
pages served) — a good way to spot clients that walk huge collections. It is
collapsed by default (a debugging aid): the Show/Hide toggle in its header
persists in the browser. At most 30 cursors are reported, most recently used
first.

Permissions editing lives on this page too (select an app for app-level
rules, a name for its overrides):

- **Action badges** — GET/POST/PUT/PATCH/DELETE/**INDEX** cycle
  allow → deny → inherit. `INDEX` is the schema-level capability to create
  or drop indexes on a collection (`POST` /
  `DELETE /q/{db}/{coll}/indexes`; *listing* indexes uses plain `GET`) —
  deliberately separate from document write/delete permissions.
  Database/collection patterns support `*` and `?` globs; deny rules are
  evaluated first (name.deny → name.allow → app.deny → app.allow
  → deny).
- The **effective rules** table shows the merged, layered result with its
  source (`name_allow`, `app_deny`, …) — wildcard patterns are flagged ⚠ so
  you always see exactly what a pattern grants.
- **Check access** lets you type any `db.coll` and see which operations are
  allowed for the selected identity.
- **Set token** rotates an app's shared credential (≥ 8 characters, hashed
  with Argon2id, 64 MiB, 3 iterations).
- **Reload from disk** re-reads `authorized_keys.yml` (it also reloads
  automatically when the file changes externally).
- A search box and **add app** cover apps that are not yet in the file.
- Every save is validated; an invalid file is rejected and the old rules
  stay active.

### Config
All other settings in one form, two sections side by side: **General**
(permission file, JWT lifetime, `/auth` throttle — the dashboard-login
throttle lives in `server.yml` — session TTL, health TTL, log level) and
**Rate limiting** (target latency, sensitivities, growth, min/max,
multiplier, tick interval, smoothing α — per-app weights are on the Clients
page).
Features:

- **Undo / Redo** — and the change history list: *click any entry to revert
  the config to the state before that change* (later changes are discarded).
- **Reload from disk** — picks up manual edits of the `config` file.
- **Reset to defaults**, **Export JSON** (backup), **Import JSON** (restore).
- The history is persisted inside the config file itself (10 000 entries).

### Logs
The server logs to rotating files on disk (`xavierdb.log`, then
`xavierdb.log.1` … — count/size from `server.yml`'s `log.files` /
`log.size_mb`, default 5 × 10 MB; restart to apply). The Logs page reads
them back, so memory stays flat regardless of traffic.

- The newest 300 lines load first; scrolling to the top loads older pages
  (300 at a time, keyed on line sequence numbers that stay stable across
  restarts and rotations).
- **Add filter** popover: level, logger, app and name facets (suggested
  from recent entries) plus a free regex — OR within a category, AND
  across categories; active filters show as removable chips.
- The header shows the retention in effect (files × MB, path).
- **Download** exports the currently loaded log view as a `.txt`.

## Dashboard API

`/dashboard/api/*` — JSON, same error shape as the client API
(`{ "error", "code", "status" }`). Every endpoint requires the `xdb_admin`
session cookie (HttpOnly, `SameSite=Strict`, `Path=/dashboard`, lifetime =
`auth.session_ttl_hours`, `Secure` added when HTTPS is on) set by
`POST /dashboard/api/login`; failed logins are throttled per IP (see
*First login* above). Sessions are in-memory: restarting the server
invalidates them.

| endpoint | description |
|---|---|
| `POST /dashboard/api/login` | `{ "username", "password" }` → sets the cookie, `{"ok":true}` |
| `POST /dashboard/api/logout` | clears the session and cookie |
| `GET /dashboard/api/session` | `{"username": …}` — current session |
| `GET /dashboard/api/metrics` | system stats, per-app/per-name RPS + p50, adaptive-limit breakdowns, cursor list |
| `POST /dashboard/api/block` / `unblock` | `{ "id" }` — `id` is `app` or `name@app` |
| `POST /dashboard/api/app_weight` | `{ "id", "weight" }` — app id only, weight 0.1–10, snapped to 0.1 |
| `GET /dashboard/api/perms` | full permission tree incl. effective rules + `version` |
| `POST /dashboard/api/perms` | replace the listed apps (rules, names, `delete`, optional `set_token`) |
| `POST /dashboard/api/perms/reload` | re-read `authorized_keys.yml` from disk |
| `GET /dashboard/api/config` | `{ version, config, history, undo_available, redo_available }` — history newest-first |
| `POST /dashboard/api/config` | `{ "config" }` — sanitized/clamped, recorded in history |
| `POST /dashboard/api/config/undo` / `redo` | `{"ok": bool}` |
| `POST /dashboard/api/config/reload` | re-read the `config` file from disk |
| `POST /dashboard/api/config/revert` | `{ "index" }` — restore the snapshot before history entry `index` (0 = newest entry, matching the history list display) |
| `POST /dashboard/api/config/reset` | back to defaults (undoable) |
| `GET /dashboard/api/config/export` | download `config.json` |
| `POST /dashboard/api/config/import` | `{ "config" }` — restore a backup |
| `GET /dashboard/api/logs` | `{ "lines": [{seq, raw, level, logger, app, name}], "total", "apps", "names", "loggers", "retention" }`; `?limit=` and `?before=<seq>` page backwards |
| `GET /dashboard/api/databases` | `{ "databases": [{name, collections}], "unavailable" }` — for the permission editor |

Notes:

- `POST /perms` touches only the apps listed in the body (it merges into
  the current file); `token_hash` is never touched unless `set_token` is
  given (token must be ≥ 8 characters). `POST /config` clamps values to
  safe ranges.
- **Known limitation:** saving permissions from the dashboard rewrites
  `authorized_keys.yml` and drops any comments it had.

## Troubleshooting

| symptom | cause / fix |
|---|---|
| dashboard login fails | wrong `admin.username`/`admin.password_hash` in `server.yml` |
| dashboard requires login again after a restart | sessions are in-memory — just log in |
| `/auth` says 401 with a correct token | the app has no `token_hash` yet → set it in Clients; or the token was just rotated |
| `403 BLOCKED` | the name or app is blocked → Clients page, unblock |
| `403 FORBIDDEN` | the identity lacks that action on that db/coll → Clients page, permissions |
| `403 FORBIDDEN` on index create/drop | the identity lacks the `INDEX` action (listing indexes only needs `GET`) |
| `504 TIMEOUT` on a `GET /q` find | the query exceeded `runtime.find_timeout_ms` (server.yml) → optimize the query/filter or raise/disable the deadline |
| `409 CONFLICT` on index create | an incompatible index with the same name/options already exists |
| `/health` is `503` | MongoDB unreachable or degraded; the body explains which |
| JWT stops working after restart | no `auth.jwt_secret` in `server.yml` → set one to keep tokens stable |
| config file corrupted | the server falls back to `config.bak` automatically |
| dashboard feels slow | open the top-bar Settings popover and lower the poll interval |
| permission edits lost their comments | the dashboard rewrites `authorized_keys.yml`; keep comments elsewhere |
