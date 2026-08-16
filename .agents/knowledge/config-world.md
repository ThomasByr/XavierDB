# Runtime state files — the "config world"

All are **cwd-relative**: the server reads/writes them relative to its working
directory (repo root bare metal; `/app` = repo mount in Docker).

| file | format | purpose | hot reload? |
|---|---|---|---|
| `.env` | dotenv | HOST, PORT, MONGODB_URI, MAX_WORKERS, MAX_INSERT_BATCH, TLS paths, USERNAME, PASSWORD_HASH (single-quoted!), JWT_SECRET, MAX_LOGINS_PER_IP_PER_MINUTE (default 5; DASHBOARD login only — /auth always uses config `auth.max_per_minute_per_ip`), LOG_FILES (1–10), LOG_SIZE_MB (1–20) | **No** — dotenvy reads at process start (with override: `.env` wins over OS env vars — required because e.g. Windows always sets `USERNAME` to the login name); restart the process (`docker compose restart api` in Docker) |
| `config` | XDB1 magic + crc32 + bincode | all tunables + history/redo/blocked | **Yes** — file watcher (500ms debounce) AND `/dashboard/api/config/reload` |
| `config.bak…` | same | automatic backup rotation (MAX_BACKUPS=5) on save; fallback on corruption | n/a |
| `authorized_keys.yml` | YAML | app credentials (Argon2id hashes) + layered permissions | **Yes** — file watcher (500ms debounce) + `/perms/reload` |
| `xavierdb.log…` | text | rotating server log: current + `xavierdb.log.1..N` (env `LOG_FILES`/`LOG_SIZE_MB`, defaults 5 × 10 MB); the Logs tab reads these files — no in-memory ring | n/a (env-only settings) |

Startup behavior (`main.rs`):
- `.env` missing → written from `include_str!("../.env.example")` (template
  compiled INTO the binary, not read from the repo). If `PASSWORD_HASH` is
  blank/unparseable → generate a strong password, Argon2id-hash it into `.env`
  (single-quoted), **print plaintext once** to stdout/logs (`docker compose
  logs api` on Docker).
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
