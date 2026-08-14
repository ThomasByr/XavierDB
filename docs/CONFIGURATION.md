# Configuration

Three places hold configuration:

| file | what lives there | edited by |
|---|---|---|
| `.env` | host/port, MongoDB URI, workers, TLS, dashboard credentials | hand |
| `authorized_keys.yml` | app tokens (Argon2id hashes) + permissions | dashboard or hand (hot reload) |
| `config` | everything else (binary) | dashboard (undo/redo) |

## `.env`

| key | default | meaning |
|---|---|---|
| `HOST` | `127.0.0.1` | bind address |
| `PORT` | `8000` | listen port |
| `MONGODB_URI` | `mongodb://localhost:27017` | MongoDB connection string |
| `MAX_WORKERS` | `4` | Tokio worker threads |
| `MAX_INSERT_BATCH` | `1000` | max documents per insert batch (`POST /q` with array `data`); must be ≥ 1, larger batches → `400` |
| `TLS_CERT_PATH` / `TLS_KEY_PATH` | empty | PEM cert + key → serve HTTPS; both files are hot-reloaded on change; ignored when invalid |
| `USERNAME` | `admin` | dashboard login name |
| `PASSWORD_HASH` | empty | Argon2id PHC hash of the dashboard password. **Must be single-quoted** (`'$argon2id$…'`) because of the `$` signs. Empty → generated once and printed to the terminal |
| `JWT_SECRET` | random per start | JWT signing secret. Set a fixed value to keep tokens valid across restarts |

## `config` (binary, auto-generated)

Created on first start (`config`, backups `config.bak`, `config.bak.2`, `config.bak.3`, …;
five backups are kept, oldest dropped).
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
| `dashboard` | `poll_seconds` | 2 | browser polling interval for metrics |
| `dashboard` | `graph_smoothing` | 5 | client-side graph smoothing window |
| `dashboard` | `theme` | `system` | `system` \| `light` \| `dark` |
| `auth` | `max_per_minute_per_ip` | 30 | brute-force throttle on `/auth` and dashboard login |
| `auth` | `session_ttl_hours` | 24 | dashboard session lifetime |
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

## `authorized_keys.yml`

See `authorized_keys.yml.example` for the full documented format. In short:

```yaml
provider1:
  token_hash: "$argon2id$…"        # shared credential for all names
  allow: [ {actions: [GET], databases: ["db1", "db*"], collections: ["*"]} ]
  deny: []
  names:
    user1: { allow: [], deny: [] } # per-name refinement; created on first login
```

Resolution order (first match wins): `name.deny` → `name.allow` → `app.deny`
→ `app.allow` → deny. Patterns are globs (`*`, `?`).

The file is watched: external edits reload live (invalid files are rejected
and the previous version stays active). Dashboard edits write the file back.
