# XavierDB

A small, fast HTTP server that exposes a **MongoDB database through a REST API**
with per-client authentication, granular permissions, adaptive load control
and a live admin dashboard.

Built in Rust (axum + tokio + mongodb driver). No Python, no Node at runtime.

```txt
POST /auth                                 client login  -> JWT (+ HttpOnly cookie)
GET|POST|PUT|PATCH|DELETE  /q/<db>/<coll>  MongoDB proxy
GET  /ls                                   list databases the caller may read (?db=<db> -> collections)
/dashboard/                                admin dashboard (login protected)
/health                                    cached health document (public)
```

## Why this shape?

- **JWT instead of a shared cookie/session table.** Verifying a JWT is one
  HMAC-SHA256 — microseconds, no disk, no shared memory, no lock. Every worker
  process can verify any token because the secret lives in process memory.
  The expensive check (Argon2id hash of the client's secret token) happens
  **once per login** on `/auth`, never on the hot `/q/` path.
- **Permissions live in `authorized_keys.yml`** (glob patterns, app-level
  inheritance, per-name overrides) and are reloaded live when the file changes.
- **Adaptive per-request limits.** Each app gets a document limit that shrinks
  when MongoDB latency or machine pressure rises and grows back when things
  calm down. Oversized requests are answered with the first page plus an
  opaque `next_cursor` (keyset pagination), so a client can never force the
  server to load a huge result set into RAM.
- **Everything tunable is tunable from the dashboard** — polling intervals,
  formula coefficients, TTLs, blocklists, permissions.

## Quick start (Docker)

Prerequisites: Docker with Compose v2 (e.g. Docker Desktop).

```bash
docker compose up --build -d
```

This builds the API image (Rust server + dashboard assets) and starts it
together with MongoDB, detached. Rebuilds are incremental — Docker reuses
cached layers (Rust dependencies, `npm ci`), so only what changed gets
rebuilt. If a stale image is ever suspected, force a full rebuild:

```bash
docker compose build --no-cache api   # no-cache is a `build` flag, not `up`
docker compose up -d
```

State persists on the host:

- MongoDB data → `${HOME}/data/xavier-mongo-db`
- API state (`.env`, `config`, `config.bak`, `authorized_keys.yml`) → the repo
  directory itself, mounted read-write over `/app`. The container uses the
  repo files directly — `config` and `authorized_keys.yml` edits are picked up
  live (hot reload); `.env` needs `docker compose restart api`.

First start:

1. The API uses the repo's `.env` as-is. If `PASSWORD_HASH` is blank, the
   admin dashboard password is **generated and printed once** in the API
   container logs (it is hashed into `.env` — `USERNAME` defaults to `admin`):

   ```bash
   docker compose logs api
   ```

2. If the repo has no valid `config`, a default binary config file is created
   in the repo.

<details>
<summary>Bare metal (no Docker)</summary>

Prerequisites: Rust (stable), a running MongoDB (default `mongodb://localhost:27017`).

```bash
npm install        # generate src/assets/app.js (dashboard TypeScript -> JS)
npm run build

cargo build --release

cp .env.example .env                  # edit HOST/PORT/MONGODB_URI if needed
cp authorized_keys.yml.example authorized_keys.yml

./target/release/XavierDB            # Windows: ./target/release/XavierDB.exe
```

First start:

1. The admin dashboard password is **generated and printed once in the
   terminal** (it is hashed into `.env` — `USERNAME` defaults to `admin`).
2. A default binary config file `config` is created.

</details>

Then:

1. Open `http://127.0.0.1:8000/dashboard/` and log in.
2. In the dashboard **Clients** view, **add app** → enter an `app_id` and a
   shared token (permissions are edited inline, per app or per name).
   (Or edit `authorized_keys.yml` by hand; the file hot-reloads.)
3. Your clients authenticate:

   ```bash
   curl -X POST http://127.0.0.1:8000/auth \
     -H "Content-Type: application/json" \
     -d '{"identifier":"user1@provider1","token":"my-secret-app-token"}'
   # -> {"token":"eyJ...","token_type":"Bearer","expires_in":5400,...}
   ```

4. Use the token on `/q/`:

   ```bash
   curl "http://127.0.0.1:8000/q/db1/items?limit=10&sort=%7B%22n%22%3A1%7D" \
     -H "Authorization: Bearer eyJ..."
   ```

The token is also returned as an HttpOnly cookie, so browsers can just log in
once per session.

## Files

| file                          | purpose                                                           |
| ----------------------------- | ----------------------------------------------------------------- |
| `.env.example`                | documented template — copy it to `.env`                           |
| `.env`                        | host/port, MongoDB URI, workers, TLS paths, dashboard credentials |
| `authorized_keys.yml`         | app credentials (Argon2id hashes) + permissions                   |
| `config`                      | binary settings file (dashboard-editable, undo/redo history)      |
| `config.bak…`                 | automatic backups of the config file                              |
| `authorized_keys.yml.example` | documented permissions template                                   |

See `docs/API_REFERENCE.md`, `docs/CONFIGURATION.md`, `docs/ADMIN_GUIDE.md` for details.

## Development

```bash
docker compose watch      # rebuilds the API image on Cargo.toml/src changes
# or, manually (incremental; clean rebuild: docker compose build --no-cache api):
docker compose up --build -d
```

<details>
<summary>Bare metal development</summary>

```bash
npm install        # only for rebuilding the dashboard TypeScript
npm run build      # compiles src/assets/ts/app.ts -> src/assets/app.js
cargo test         # unit tests (config, permissions, cursors, auth)
cargo run
```

</details>

The dashboard is embedded into the binary at compile time — no external files
needed at runtime.
