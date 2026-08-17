# Credentials — machine-local secrets & regeneration recipes

**Actual credentials are NEVER in repo files.** They are machine-local and
live in `.pi/notes/credentials.md` (gitignored via `.pi/.gitignore` — it
contains `*`; read it when you need them). Never commit or copy secrets into
AGENTS.md, `.agents/`, or any doc. That file also holds machine-local
dev-environment notes (portable-mongod layout, test dbs, log paths, jsdom
harness location).

If `.pi/notes/credentials.md` is missing (new machine, wiped notes), obtain
or regenerate credentials as follows:

- **Dashboard password** — the plaintext is printed EXACTLY ONCE, at first
  bootstrap, in the server log: bare metal = the terminal/stdout the server
  was started with (e.g. `/tmp/xdb.log`); Docker = `docker compose logs api`.
  `server.yml` only ever holds the Argon2id `admin.password_hash` (not
  reversible). To force a fresh password: blank `admin.password_hash` in
  `server.yml` (or copy `server.yml.example` over it) and restart the server —
  a new password is generated and printed once. `admin.username` comes from
  `server.yml` (default `admin`).
- **Client app tokens** (`identifier` = `name@app`, shared secret token) —
  `authorized_keys.yml` stores only the Argon2id `token_hash`, so the
  plaintext is NOT recoverable. If lost, reset via the dashboard (Clients
  view → add app / perms editor → set token, min 8 chars) or rewrite the yml
  entry with a freshly hashed token. To hash one:
  `uv run --with argon2-cffi python -c "..."` (Argon2id PHC) — verify it
  against the SERVER (swap `token_hash` in authorized_keys.yml → watcher
  reload → /auth), not against the library (argon2-cffi's verify has been
  observed broken in some environments).
- **TLS certs** — paths are `tls.cert_path`/`tls.key_path` in `server.yml`
  (or `TLS_CERT_PATH`/`TLS_KEY_PATH` env vars);
  regenerate with openssl (self-signed is fine for dev). MSYS-shell trap:
  openssl may mangle `-subj "/CN=..."` via MSYS path conversion — use
  `MSYS_NO_PATHCONV=1` AND Windows-style output paths (the two don't mix).
- **MongoDB** — URI in `server.yml` (`network.mongodb_uri`, default
  `mongodb://localhost:27017`); install/discovery per
  knowledge/toolchain.md. A portable no-admin-rights setup (zip + explicit
  dbpath + localhost bind) is documented in `.pi/notes/credentials.md`.

Also remember: `server.yml` may be awkward to touch from some shells (a
protected path on the dev machine) — read it via `read`/`cat` with care;
`$` in `admin.password_hash` needs no quoting in YAML.
