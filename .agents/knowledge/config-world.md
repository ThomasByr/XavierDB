# Runtime state files — the "config world"

All are **cwd-relative**: the server reads/writes them relative to its working
directory (repo root bare metal; `/app` = repo mount in Docker).

| file | format | purpose | hot reload? |
|---|---|---|---|
| `server.yml` | YAML | startup-only server settings (see `server.yml.example`): tls.cert_path/key_path, network.host/port/mongodb_uri/trust_proxy_headers, runtime.max_workers/max_insert_batch/find_timeout_ms (find deadline for GET /q: default 10 000 ms, 0 = disabled, nonzero clamped 100..=3 600 000, env `FIND_TIMEOUT_MS`), runtime.keyset_type_brackets (`"all"|"id-only"|"off"`, default `all`, env `KEYSET_TYPE_BRACKETS`, invalid → WARN + `all`; controls the keyset-pagination `$type` fallback branches — see architecture.md §cursor pagination + known-limits.md for the safety contract), log.files(1–10)/size_mb(1–20), admin.username/password_hash/max_logins_per_ip_per_minute (default 5; DASHBOARD login only — /auth always uses config `auth.max_per_minute_per_ip`), auth.jwt_secret. Precedence: **env var > server.yml > baked-in default** (compose injects `HOST`/`MONGODB_URI`/`TRUST_PROXY_HEADERS`; bare metal uses the file/defaults). Env-only override NOT in server.yml: `DISK_PATH` (dashboard disk metric target filesystem — default app cwd; empty counts as unset, see architecture.md §5 system sampling). EXCEPTION: `admin.username`/`admin.password_hash` always come from the file (Windows always sets `USERNAME`). `$` needs no quoting in YAML. | **No** — read once at boot; restart the process (`docker compose restart xavierdb` in Docker) |
| `.env` | dotenv | **Docker Compose ONLY** — `UID`/`GID` for the `user: "$UID:$GID"` interpolation. The app never reads it (server.yml replaced it). | n/a (compose) |
| `config` | XDB1 magic + crc32 + bincode | all tunables + history/redo/blocked | **Yes** — file watcher (500ms debounce) AND `/dashboard/api/config/reload` |
| `config.bak…` | same | automatic backup rotation (MAX_BACKUPS=5) on save; fallback on corruption | n/a |
| `authorized_keys.yml` | YAML | app credentials (Argon2id hashes) + layered permissions | **Yes** — file watcher (500ms debounce) + `/perms/reload` |
| `xavierdb.log…` | text | rotating server log: current + `xavierdb.log.1..N` (server.yml log.files/log.size_mb, defaults 5 × 10 MB); the Logs tab reads these files — no in-memory ring | n/a (startup-only settings) |

Startup behavior (`main.rs` + `settings.rs`):
- `server.yml` missing → written from `include_str!("../server.yml.example")`
  (template compiled INTO the binary, not read from the repo). If
  `admin.password_hash` is blank/unparseable → generate a strong password,
  Argon2id-hash it, write it back into `server.yml`, **print plaintext once**
  to stdout/logs (`docker compose logs xavierdb` on Docker).
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