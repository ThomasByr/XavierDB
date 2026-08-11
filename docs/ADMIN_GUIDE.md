# Admin guide

The dashboard lives at `/dashboard/` (or `https://…` when TLS is enabled).
It is a single static page: no libraries, no build step at runtime — the
TypeScript is compiled at build time and embedded in the binary. Four
hash-routed views: `#/overview`, `#/clients`, `#/config`, `#/logs`.

## First login

- Username: `USERNAME` from `.env` (default `admin`).
- Password: if `PASSWORD_HASH` is empty, the server **generates a strong
  64-character password on startup, prints it once in the terminal** and
  stores only its Argon2id hash in `.env`. Change it by editing `.env`
  (put the PHC hash in single quotes) and restarting.

Sessions are in-memory and last `auth.session_ttl_hours` (default 24 h);
a restart logs everyone out. Login attempts are throttled per IP
(`auth.max_per_minute_per_ip`, default 30/min, shared with the client
`/auth` endpoint).

## Pages

### Overview
System metrics polled every `dashboard.poll_seconds` (default 2 s):
CPU, memory, disk, network (KB/s), plus QPS, MongoDB ping and uptime.
Graphs are smoothed client-side (`dashboard.graph_smoothing`). Polling
interval, smoothing window and theme are configurable on the Config page —
the golden rule: *anything that could need tuning is tunable here*.

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
pages served) — a good way to spot clients that walk huge collections. The
card can be hidden with the toggle in its header.

Permissions editing lives on this page too (select an app for app-level
rules, a name for its overrides):

- **Action badges** — GET/POST/PUT/PATCH/DELETE cycle allow → deny →
  inherit. Database/collection patterns support `*` and `?` globs; deny
  rules are evaluated first (name.deny → name.allow → app.deny → app.allow
  → deny).
- The **effective rules** table shows the merged, layered result with its
  source (`name_allow`, `app_deny`, …) — wildcard patterns are flagged ⚠ so
  you always see exactly what a pattern grants.
- **Check access** lets you type any `db.coll` and see which operations are
  allowed for the selected identity.
- **Set token** rotates an app's shared credential (hashed with Argon2id,
  64 MiB, 3 iterations).
- **Reload from disk** re-reads `authorized_keys.yml` (it also reloads
  automatically when the file changes externally).
- A search box and **add app** cover apps that are not yet in the file.
- Every save is validated; an invalid file is rejected and the old rules
  stay active.

### Config
All other settings in one form: General (permission file, JWT lifetime,
auth throttle, session TTL), **Rate limiting** (target latency,
sensitivities, growth, min/max, multiplier, tick interval, smoothing α —
per-app weights are on the Clients page), Health (TTL) and Dashboard
(polling, smoothing, theme). Features:

- **Undo / Redo** — and the change history list: *click any entry to revert
  the config to the state before that change* (later changes are discarded).
- **Reload from disk** — picks up manual edits of the `config` file.
- **Reset to defaults**, **Export JSON** (backup), **Import JSON** (restore).
- The history is persisted inside the config file itself (10 000 entries).

### Logs
In-memory ring of the last ~1500 server log lines (info/warn/error),
with download as a `.txt`.

## Dashboard API

`/dashboard/api/*` — JSON, same error shape as the client API
(`{ "error", "code", "status" }`). Every endpoint requires the `xdb_admin`
session cookie (HttpOnly, `SameSite=Strict`, `Path=/dashboard`, 24 h) set
by `POST /dashboard/api/login`; failed logins are throttled per IP.
Sessions are in-memory: restarting the server invalidates them.

| endpoint | description |
|---|---|
| `POST /dashboard/api/login` | `{ "username", "password" }` → sets the cookie, `{"ok":true}` |
| `POST /dashboard/api/logout` | clears the session and cookie |
| `GET /dashboard/api/session` | `{"username": …}` — current session |
| `GET /dashboard/api/metrics` | system stats, per-app/per-name RPS + p50, adaptive-limit breakdowns, cursor list |
| `POST /dashboard/api/block` / `unblock` | `{ "id" }` — `id` is `app` or `name@app` |
| `POST /dashboard/api/app_weight` | `{ "id", "weight" }` — weight 0.1–10, snapped to 0.1 |
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
| `GET /dashboard/api/logs` | `{ "lines": […] }` — the log ring buffer |
| `GET /dashboard/api/databases` | `{ "databases": [{name, collections}], "unavailable" }` — for the permission editor |

Notes:

- `POST /perms` touches only the apps listed in the body (it merges into
  the current file); `token_hash` is never touched unless `set_token` is
  given. `POST /config` clamps values to safe ranges.
- **Known limitation:** saving permissions from the dashboard rewrites
  `authorized_keys.yml` and drops any comments it had.

## Troubleshooting

| symptom | cause / fix |
|---|---|
| dashboard login fails | wrong `USERNAME`/`PASSWORD_HASH` in `.env`; the hash must be single-quoted (it contains `$`) |
| dashboard requires login again after a restart | sessions are in-memory — just log in |
| `/auth` says 401 with a correct token | the app has no `token_hash` yet → set it in Clients; or the token was just rotated |
| `403 BLOCKED` | the name or app is blocked → Clients page, unblock |
| `403 FORBIDDEN` | the identity lacks that action on that db/coll → Clients page, permissions |
| `/health` is `503` | MongoDB unreachable or degraded; the body explains which |
| JWT stops working after restart | no `JWT_SECRET` in `.env` → set one to keep tokens stable |
| config file corrupted | the server falls back to `config.bak` automatically |
| dashboard feels slow | lower `poll_seconds` (Config) — or raise it to save resources |
| permission edits lost their comments | the dashboard rewrites `authorized_keys.yml`; keep comments elsewhere |
