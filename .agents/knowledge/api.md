# API reference (condensed — full details in docs/API_REFERENCE.md and docs/ADMIN_GUIDE.md)

## Client API

| route | auth | behavior |
|---|---|---|
| `POST /auth` | public (throttled) | login → JWT + cookie (see architecture.md Auth) |
| `GET /q/{db}/{coll}` | Bearer or cookie | query: `filter`/`sort`/`projection` URL-encoded JSON, `limit`, `cursor`; keyset pagination |
| `POST /q/{db}/{coll}` | Bearer | insert (no filter) / update (filter) — data auto-`$set` |
| `PUT /q/{db}/{coll}` | Bearer | update, 404 if 0 matched |
| `PATCH /q/{db}/{coll}` | Bearer | upsert (200 updated / 201 inserted); array `data` = upsert-many (200) |
| `DELETE /q/{db}/{coll}` | Bearer | `{filter}` → `{deleted_count}`, 404 if 0 |
| `GET /ls` | Bearer | flat list of listable dbs; `?db=X` → collections |
| `GET /health` | public | health doc (+ `constants.max_insert_batch`, `constants.jwt_token_lifetime_seconds`, `constants.max_document_limit`); 200 ok / 503 otherwise |

Errors: `{error, code, status}`; codes BAD_REQUEST/INVALID_FILTER/INVALID_SORT/
INVALID_LIMIT/INVALID_CURSOR (400), UNAUTHORIZED (401), FORBIDDEN/BLOCKED (403),
NOT_FOUND (404), CONFLICT (409, duplicate key), TOO_MANY_REQUESTS (429),
INTERNAL_ERROR (500), UNAVAILABLE (503). Messages sanitized (paths,
IPv4/IPv6 scrubbed; bare hostnames/host:port are NOT — they're deployment
config). Client-caused Mongo command errors (bad regex, malformed shapes,
validation) map to 400; duplicate keys → 409.

## Dashboard API (condensed)

- `POST /dashboard/api/login` `{username, password}` → `{"ok":true}` + cookie
  `xdb_admin` (Path=/dashboard, HttpOnly, SameSite=Strict, Max-Age follows
  `auth.session_ttl_hours`; +`; Secure` under TLS). Throttle SEPARATE from
  client /auth: per-IP, default 5/min from env `MAX_LOGINS_PER_IP_PER_MINUTE`
  (dashboard login only). Argon2id verify runs on the blocking
  pool; unknown usernames verify against a fixed dummy hash (no timing
  oracle). Success/failure logged (info/warn).
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
  1..=130 chars) → mutates `config.blocked` with history (desc "block {id}",
  path "blocked").
- `POST /dashboard/api/app_weight` `{id, weight}` — 0.1..=10, snapped to 0.1;
  path "rate_limit.weights.{id}".
- `GET /dashboard/api/perms` → `{version, apps:[{app, token_set, allow, deny,
  effective:[{source, actions, databases, collections}], names:[...]}]}`.
  Rule = `{actions, databases, collections}` (collections defaults ["*"]).
  EffectiveRule source: name_deny|name_allow|app_deny|app_allow.
  `POST /dashboard/api/perms` — MERGE semantics: only listed apps touched;
  app-level allow/deny REPLACED wholesale per app (`entry.allow = a.allow`
  unconditionally → omitting a field CLEARS it — setup clients must send the
  full arrays); `delete:true` removes app/name; `set_token` (min 8 chars)
  rehashes (Argon2id, ~5 s). Unknown JSON fields ignored — a GET snapshot can
  be POSTed verbatim. `POST /perms/reload` re-reads yml.
- `GET /dashboard/api/config` → `{version, config, history (NEWEST-first),
  undo_available, redo_available}`; `POST /config` sanitizes/clamps (exact
  ranges in architecture.md Config); undo/redo/reload (fallback to config.bak
  on corruption, returns `warning`)/revert `{index}` (newest-first display
  position)/reset/export (JSON attachment)/import. undo/redo/reload/reset are
  POST-with-NO-body (`{}` → 400). Config mutations hot-apply log_level via
  `state::apply_log_level` (reload::Layer — no restart).
- `GET /dashboard/api/logs` → `{lines:[{seq, raw, level, logger, app, name}],
  total, apps, names, loggers, retention:{files, size_mb, path}}` — reads the
  ROTATING LOG FILES (xavierdb.log + .1..N, env LOG_FILES/LOG_SIZE_MB, no
  in-memory ring); `?limit=N` (0 = all, cap 10k), `?before=<seq>` load-older
  paging; `apps`/`names`/`loggers` = facets from a bounded scan (last 2000
  lines); `names` = [{app, name}] pairs sorted by app. EVERYTHING the process
  emits lands in the files (tracing events, eprintln/println, panics via a
  custom hook). Logs SURVIVE restarts, and `seq` (a global line number seeded
  by a startup scan) stays stable across restarts AND rotations.
- `GET /dashboard/api/databases` → `{databases:[{name, collections}],
  unavailable}` — admin-only, unfiltered (client-side equivalent: `/ls`).

## Static UI serving

- `/dashboard`, `/dashboard/`, `/dashboard/{*rest}` → embedded SPA (index.html,
  app.js, styles.css) served no-cache via include_str! (compile-time).
  NOTE: `/dashboard/index.html` itself 404s — only `/dashboard/` serves the
  shell.
