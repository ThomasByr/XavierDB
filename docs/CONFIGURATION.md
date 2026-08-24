# Configuration

Three places hold configuration:

| file | what lives there | edited by |
|---|---|---|
| `server.yml` | host/port, MongoDB URI, workers, TLS, dashboard credentials, JWT secret | hand (startup-only) |
| `authorized_keys.yml` | app tokens (Argon2id hashes) + permissions | dashboard or hand (hot reload) |
| `config` | everything else (binary) | dashboard (undo/redo) |

Precedence: OS environment variable (set and non-empty) > `server.yml` >
baked-in default — so Docker Compose can inject container values without
touching the file. Recognized overrides: `HOST`, `PORT`, `MONGODB_URI`,
`TRUST_PROXY_HEADERS`, `TLS_CERT_PATH`, `TLS_KEY_PATH`, `MAX_WORKERS`,
`MAX_INSERT_BATCH`, `FIND_TIMEOUT_MS`, `KEYSET_TYPE_BRACKETS`, `LOG_FILES`,
`LOG_SIZE_MB`, `MAX_LOGINS_PER_IP_PER_MINUTE`, `JWT_SECRET`. Exception:
`admin.username` and `admin.password_hash` always come from the file
(Windows always sets `USERNAME` in the environment). `server.yml` is read
once at boot; restart to apply (TLS cert/key **files**, however, are
watched and hot-reloaded). Copy `server.yml.example` to `server.yml` to
get a documented template.

## `server.yml`

| key | default | meaning |
|---|---|---|
| `tls.cert_path` / `tls.key_path` | empty | PEM cert + key → serve HTTPS; both files are hot-reloaded on change; ignored when invalid |
| `network.host` | `127.0.0.1` | bind address |
| `network.port` | `8000` | listen port |
| `network.mongodb_uri` | `mongodb://localhost:27017` | MongoDB connection string |
| `network.trust_proxy_headers` | `false` | trust `X-Real-IP` / the last `X-Forwarded-For` entry as the client IP for per-IP throttling (falls back to the socket peer on malformed values) |
| `runtime.max_workers` | `4` | Tokio worker threads |
| `runtime.max_insert_batch` | `1000` | max documents per insert batch (`POST /q` with array `data`); must be ≥ 1, larger batches → `400` |
| `runtime.find_timeout_ms` | `10 000` | server-side deadline for `GET /q` finds; exceeded → `504 TIMEOUT`. `0` disables; otherwise clamped 100–3 600 000 ms |
| `runtime.keyset_type_brackets` | `all` | keyset-pagination type-bracket mode: `all` (correct for mixed-type data), `id-only` (drop the `$type` fallback branches for the `_id` column — recommended when every `_id` per collection is a single BSON type; without it every deep paginated page over an `_id`-sorted collection is a full `_id` index scan), `off` (drop them everywhere — only when sort fields are single-typed too, or pagination can silently skip documents). Invalid values fall back to `all` with a WARN |
| `log.files` / `log.size_mb` | `5` / `10` | rotating log files (clamped 1–10 files × 1–20 MB) |
| `admin.username` | `admin` | dashboard login name |
| `admin.password_hash` | empty | Argon2id PHC hash of the dashboard password. `$` needs no quoting in YAML. Empty → generated once and printed to the terminal |
| `admin.max_logins_per_ip_per_minute` | `5` | dashboard-login brute-force throttle per IP per minute (clamped 1–10 000); **applies only to `POST /dashboard/api/login`** — `/auth` has its own throttle, `config.auth.max_per_minute_per_ip` |
| `auth.jwt_secret` | random per start | JWT signing secret. Set a fixed value to keep tokens valid across restarts |

## `config` (binary, auto-generated)

Created on first start (`config`, backups `config.bak`, `config.bak.2`, `config.bak.3`, …;
five backups are kept, oldest dropped). The file also carries metadata
(`version`, `created_at`, `last_modified`) and the undo/redo history stacks
themselves (both capped at 10 000 entries).
The dashboard **Config** page edits it live; every change is recorded in a
10 000-entry undo history (with redo, and click-to-revert on any entry).
History snapshots are flat (the values only — they never embed the history
itself), so the file grows linearly with the number of changes.
Values saved from the dashboard are clamped to safe ranges, and the file is
checksummed (CRC32) and written atomically.

| section | field | default | meaning |
|---|---|---|---|
| `global` | `jwt_token_lifetime_minutes` | 90 | how long a JWT stays valid |
| `global` | `permission_file` | `authorized_keys.yml` | permissions file path |
| `rate_limit` | `min_limit` / `max_limit` | 1 / 200 | bounds of the per-page document limit |
| `rate_limit` | `multiplier` | 1.0 | master dial applied to every app's limit |
| `rate_limit` | `target_latency_ms` | 50 | p50 processing target; above it limits shrink |
| `rate_limit` | `latency_sensitivity` | 1.0 | how hard latency overshoot pulls limits down |
| `rate_limit` | `pressure_sensitivity` | 1.5 | how hard CPU/RAM pressure pulls limits down |
| `rate_limit` | `growth_rate` | 1.15 | per-tick recovery when healthy |
| `rate_limit` | `tick_seconds` | 5 | recomputation interval |
| `rate_limit` | `ema_alpha` | 0.2 | smoothing of the request-rate measurement |
| `rate_limit` | `weights` | — | per-app weight multiplier (0.1–10, snapped to 0.1) — set per app in the dashboard Clients view |
| `health` | `cache_ttl_seconds` | 5 | /health refresh interval |
| `dashboard` | `log_level` | `info` | console + log-file verbosity: `info` \| `debug` (debug adds one line per `/q`/`/ls` request: method, path, identity) — hot-reloadable |
| `auth` | `max_per_minute_per_ip` | 30 | brute-force throttle on `/auth` (the dashboard login has its own throttle — `admin.max_logins_per_ip_per_minute` in `server.yml`) |
| `auth` | `session_ttl_hours` | 24 | dashboard session lifetime (clamped 1–720) |
| `blocked` | list | — | blocked `name@app` or bare `app` identifiers |

### The adaptive limit formula

Every `tick_seconds`, for each active app:

```
lat_err   = max(0, (p50_ms − target_latency_ms) / target_latency_ms)
pressure  = max(0, (cpu% − 60)/40, (mem% − 70)/30)
shrink    = 1 / (1 + latency_sensitivity·lat_err + pressure_sensitivity·pressure)
internal  = internal × shrink          (or × growth_rate when shrink ≥ 1)
enforced  = clamp(round(internal × multiplier × weight), min_limit, max_limit)
```

Every `/q/` GET is served at most `min(requested, enforced)` documents; when
the enforced limit bites, the response carries a `next_cursor`.

`internal` starts at `max_limit` on the first tick; both the growth and the
shrinkage branch keep `internal` clamped between `min_limit` and `max_limit`.

Dashboard edits, config import and revert all sanitize the values before they
are persisted: `min_limit` is clamped to 1–10 000 and `max_limit` to
[min_limit, 10 000], so the invariant `min_limit ≤ max_limit` always holds
(the metrics loop clamps with both). `load_from_disk` raises `max_limit` to
`min_limit` on load if the invariant is ever violated on disk.

Other clamps: `multiplier` 0.05–20, `target_latency_ms` 1–60 000,
`latency_sensitivity` / `pressure_sensitivity` 0–20, `growth_rate` 1–2,
`tick_seconds` 1–3600, `ema_alpha` 0.01–0.9, weights 0.1–10,
`cache_ttl_seconds` 1–3600, `jwt_token_lifetime_minutes` 1–43 200; invalid
`log_level` values fall back to `info`.

Dashboard theme, chart smoothing coefficient and metrics poll interval are
**per-browser** preferences (localStorage), set from the dashboard top bar
(theme toggle + Settings popover) — they are not part of the server config.

## `authorized_keys.yml`

See `authorized_keys.yml.example` for the full documented format. In short:

```yaml
provider1:
  token_hash: "$argon2id$…"        # shared credential for all names
  allow: [ {actions: [GET], databases: ["db1", "db*"], collections: ["*"]} ]
  deny: []
  names:
    user1:
      allow: [ {actions: [GET, INDEX], databases: ["db1"], collections: ["*"]} ]
      deny: []                    # per-name refinement; created on first login
```

Actions: `GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `INDEX`. `INDEX` is the
schema-level capability to ensure/drop indexes
(`POST`/`DELETE /q/{db}/{coll}/indexes`; *listing* indexes uses plain
`GET`) — deliberately separate from document write/delete permissions.

Resolution order (first match wins): `name.deny` → `name.allow` → `app.deny`
→ `app.allow` → deny. Patterns are globs (`*`, `?`).

The file is watched: external edits reload live (invalid files are rejected
and the previous version stays active). Dashboard edits write the file back.
