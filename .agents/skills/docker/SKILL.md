---
name: docker
description: Docker/compose deployment for XavierDB — the DEFAULT way to run, build, deploy AND test. xdb-compose.sh up|watch|build|logs|restart|ps|down|deploy|battery, plus how MongoDB is always launched via Docker and the inotify/watcher_reload limitation on Docker Desktop. Use for managing the API + MongoDB container stack.
---

# Docker / compose deployment — VERIFIED 2026-08-16 on Docker Desktop 29.7.2 (WSL2)

> **Script:** `xdb-compose.sh` (same dir) — `up | build [--no-cache] |
> logs [-f] | password | restart | ps | down | deploy | battery | mongo`. Prefer it over
> hand-typed commands; defaults overridable via `XDB_*` env (see
> `.agents/settings/defaults.sh`).

**This stack is the DEFAULT for running, building, deploying AND testing** —
tests run against these containers unless Docker itself is broken (then
`skills/docker-fallback/SKILL.md`; MongoDB STAYS in Docker even there).

**MongoDB is ALWAYS launched with Docker:** the compose `mongodb` service
(`docker compose up -d mongodb`), or a standalone `docker run -p 27017:27017
mongo:8.0` when you need it on the host network without the stack. Never
start a bare-metal `mongod` on the toolchain path.

First real run: `docker compose up --build -d` worked end to end on the dev
machine (Docker Desktop 29.7.2, WSL2 backend, overlayfs). The integration
battery (local rust, `cargo test`) against the docker stack is 180/182 —
the 2 watcher_reload failures are a Docker-Desktop-only limitation, see
"inotify / file watchers" below. Re-verified 2026-08-17 against the
rebuilt image (post build-speed fixes + dummy-binary fix): 108/110 as
counted then; re-verified 2026-08-18 (indexes endpoints): same 2 failures,
180/182.
Note for running the battery: on Windows the PATH `bash` is a broken WSL
stub — invoke git-bash explicitly for tests/bootstrap.sh
(`& "C:\Program Files\Git\bin\bash.exe" tests/bootstrap.sh ...`).

## Ops commands

The stack is ALWAYS base + one overlay, passed explicitly (`-f`) so no
implicit override file is ever auto-merged:

```bash
# via the script (XDB_ENV=dev|pre|prod, default pre — see defaults.sh):
xdb-compose.sh up                     # docker compose -f compose.base.yaml -f compose.<env>.yaml up --build -d

# by hand:
docker compose -f compose.yaml -f compose.pre.yaml up --build -d
docker compose -f compose.yaml -f compose.dev.yaml up -d --build   # dev cargo-watch loop
docker compose -f compose.yaml -f compose.prod.yaml ...            # prod (deploy.sh)
docker compose -f compose.yaml -f compose.pre.yaml build --no-cache xavierdb   # force full rebuild
docker compose -f compose.yaml -f compose.pre.yaml logs xavierdb    # first-run dashboard password
docker compose -f compose.yaml -f compose.pre.yaml restart xavierdb # needed after server.yml changes
```

## Compose file layout (base + overlays, since 2026-08-21)

- `compose.base.yaml` — BASE, never used alone: both services, image/build,
  healthchecks, depends_on, ulimits, 127.0.0.1 port bindings, HOST/
  MONGODB_URI env, the `.:/app` repo mount, `xavier_configdb`. NO resource
  limits, NO mongo data mount, NO develop.watch, NO TRUST_PROXY_HEADERS.
- `compose.dev.yaml` — cargo-watch loop (Dockerfile.dev image
  `xavierdb-api-dev`), NO resource limits (in-container builds use whatever
  the host has; test DBs are small), mongo named volume + default WT cache,
  faster healthcheck.
- `compose.pre.yaml` (script DEFAULT) — prod-shaped: prod image, no watch,
  mongo 4g/5g-swap/WT 2.0, API 1g/1.5g-swap, named volume `xavier_mongo_db`,
  literals (pinned per env, not env-var tunable).
- `compose.prod.yaml` — the 8 GB OVH VPS: mongo 7g/8.5g-swap/WT 3.5 (defaults
  of the `${XAVIER_*}` interpolation vars, tunable via `.env`), API
  1g/1.5g-swap, **host bind mount** `${XAVIER_MONGO_DATA:-${HOME}/data/
  xavier-mongo-db}:/data/db`, TRUST_PROXY_HEADERS=true. Mongo runs with the
  image default user here (root entrypoint chowns the bind mount, drops to
  mongodb) — dev/pre keep the `user: ":"` hack. MIGRATION NOTE in the file:
  prod ran on the named volume before; switching needs a one-time data copy.
- `compose.example.yaml` — STANDALONE end-user template (base+prod merged,
  `${XAVIER_*}`-parameterized, bind default `./data/mongo`): users copy it to
  `compose.yaml` (gitignored, local copy) and tune `.env` (see `.env.example`). Never auto-loaded.
- `compose.override.yaml` is GONE (and `docker compose watch` / `develop.watch`
  with it — the cargo-watch dev loop replaced image-rebuild watch; `xdb-
  compose.sh watch` was removed).
- Merge rules that matter (verified 2026-08-21 via `docker compose config`):
  scalars/maps overridden by the later file; volumes merge by container
  target path (pre/prod re-mounting `/data/db` replaces any base/dev mount);
  ports concatenate (identical everywhere → base only).

## compose file facts (verified)

## compose file facts (verified)

- xavierdb has **no `image:` key** (build-only; with both, compose would tag the
  build and clobber the official `rust:1-slim-bookworm` tag).
- Mongo healthcheck (marker-file hack, retries 100 × 5s) and the xavierdb
  healthcheck (curl /health, https fallback) both verified green.
- **Env precedence (since 2026-08-16): env var > `server.yml` > default.**
  The app no longer reads `.env` at all — all startup settings live in
  `server.yml` (YAML, startup-only, no hot reload), shared by bare metal and
  Docker through the repo mount. Compose's `HOST=0.0.0.0` and
  `MONGODB_URI=mongodb://mongodb:27017` env vars OVERRIDE the file, so the
  same server.yml with bare-metal defaults (127.0.0.1 / localhost) works in
  both worlds. EXCEPTION: `admin.username`/`admin.password_hash` always come
  from the file (Windows always sets `USERNAME` — an env override would
  silently break the dashboard login on bare metal). `.env` now holds
  UID/GID (compose `user:` interpolation) + the `XAVIER_*` prod resource
  vars (compose interpolation in compose.prod.yaml / compose.example.yaml
  ONLY — dev/pre overlays use literals; the app never reads .env).
- `user: ":"` (mongodb) and `user: "$UID:$GID"` (xavierdb, from .env UID/GID=1000)
  both run fine on Docker Desktop; the repo mount stays transparently
  editable from Windows.
- Mongo volume: per-env (see the layout section above) — named volume
  `xavier_mongo_db` for dev/pre, host bind mount for prod/example.
- First boot with no `server.yml` in the repo: the server creates it from the
  embedded `server.yml.example` template and (blank `admin.password_hash`)
  generates a dashboard password printed once — `docker compose logs xavierdb`.

## inotify / file watchers (the one real limitation, verified)

The perms/config/TLS hot reloads are **pure inotify** (`watch_file` in
src/main.rs, no polling fallback). Over a Docker Desktop bind mount
(VirtioFS) **no inotify events are delivered at all** — verified: even a
write done INSIDE the container to a freshly-watched file never fires the
watcher (virtiofsd implements no FUSE notify). Consequences:

- `cargo test` against a Docker-Desktop API: `watcher_reload` fails
  (`perms_file_watcher_reload` at the "watcher picked up the appended app"
  assert; `reload_endpoints` then dies on the poisoned suite lock — it passes
  standalone). Everything else is green (180/182, re-verified 2026-08-20 with runtime.keyset_type_brackets=id-only in server.yml).
  Live A/B proof: a host-side `touch authorized_keys.yml` fires NO reload
  log line in `docker logs xavierdb`, while the identical battery run on bare
  metal logs `authorized_keys.yml reloaded from disk` for both watcher
  tests.
- On bare metal (Linux/Windows host kernel inotify) and on real Linux Docker
  hosts (kernel bind mount) the watchers work — the battery was green there
  before.
- Dashboard/perms writes still WORK in docker (the API reloads its own
  writes directly); only external file edits go unnoticed. Manual fix:
  `docker compose restart xavierdb`, or use the `/perms/reload` +
  `/config/reload` endpoints (relay works in docker).

## Dockerfile facts (verified 2026-08-17, after the build-speed fixes)

- Stage 1 node:22-bookworm-slim: `npm ci` (lockfile), `npm run build`
  (esbuild ts/app.ts → src/assets/app.js — the only generated asset;
  index.html/styles.css are static, come from the context).
- Stage 2 rust:1-slim-bookworm: apt install cmake perl pkg-config
  (aws-lc-sys needs them; gcc+libc6-dev already in the image) + curl
  ca-certificates (healthcheck); dummy-main layer copies `Cargo.toml
  Cargo.lock` (the lockfile is REQUIRED — without it the dummy resolves
  "latest" and the real build recompiles ~30 crates); both cargo steps use
  `--mount=type=cache` for `/usr/local/cargo/registry` (downloads) AND a
  shared `id=xavier-target` target cache; `COPY . .` then overlay app.js
  from the node stage; binary → /usr/local/bin/XavierDB;
  WORKDIR /app; CMD ["XavierDB"].
- include_str! needs src/assets/{index.html,styles.css,app.js} + server.yml.example
  in the build context at compile time. TRAP: a `server.yml*` pattern in
  .dockerignore matches `server.yml.example` too → real build fails; the
  pattern must be the exact name `server.yml` (fixed 2026-08-17; was masked
  before because the dummy-binary bug meant the real crate never compiled).
- Image NEVER contains state files (.dockerignore + COPY lands in image
  root /; /app is empty in the image) — see knowledge/repo-layout.md.

## Build speed — fixes APPLIED & VERIFIED 2026-08-17

Fast-rebuild pipeline verified end to end (Docker Desktop 29.7.2): no-op
rebuild ~2 s; src-only rebuild ≈ 13 s (only the app crate recompiles, deps
come from the target cache mount); image binary is the REAL one
(19.6 MB) and `docker compose up -d` starts healthy with normal logs.

1. **Build context is now ~5 kB** — the full `.dockerignore` excludes
   `.agents/ .github/ .git/ .idea/ .pi/ .vscode/ assets/ docs/ examples/
   node_modules/ target/ tests/ web/` plus dotfiles/state/log files and
   `*.md *.swp *.tmp` (the old bare `target` pattern only matched the ROOT
   directory; `examples/target` alone was 1.46 GB of the 1.6 GB context).

2. **Dependencies compile once, not twice** — the dummy layer copies
   `Cargo.lock` along with `Cargo.toml` (see Dockerfile facts above), and
   both cargo steps share the `xavier-target` cache mount + a
   `/usr/local/cargo/registry` cache mount.

3. **CRITICAL TRAP with the shared target cache mount (fixed, keep the
   fix):** the dummy step leaves `target/release/XavierDB` (the 437 KB
   `fn main() {}` stub) in the cache mount, with a NEWER mtime than the
   `COPY . .` sources (COPY preserves context mtimes). Cargo then sees a
   "fresh" bin in the real step, skips compiling, and `cp` ships the DUMMY
   → container exits 0 instantly, empty logs, restart loop (observed live
   twice on 2026-08-17). Fix (VERIFIED): end the dummy step with
   `cargo clean -p XavierDB --release` to purge ONLY this crate's
   artifacts (deps stay cached) — it MUST run BEFORE `rm -rf src`
   (cargo clean -p needs src/ to resolve the package; reversed it exits
   101 and the build fails). If a rebuild finishes suspiciously fast and
   the container crash-loops with no logs, check
   `docker run --rm --entrypoint ls <img> -la /usr/local/bin/XavierDB`:
   ~437 KB = dummy, ~19.6 MB = real.

## deploy.sh & the dev loop (restructured 2026-08-21)

- `deploy.sh` (repo root, for the Linux prod host): `set -euo pipefail`,
  `git pull origin main` → `docker compose -f compose.base.yaml -f
  compose.prod.yaml build xavierdb` → `... up -d`. It pins the PROD pair.
  Prod resource tuning lives in the VPS's `.env` (`XAVIER_*` vars, defaults
  in the file are the 8 GB VPS values); the mongo data path is
  `XAVIER_MONGO_DATA` (default `${HOME}/data/xavier-mongo-db`).
  ONE-TIME MIGRATION (old stack ran mongo on the named volume
  `xavierdb_xavier_mongo_db`): the copy-out recipe is in a comment block at
  the top of `compose.prod.yaml` — run it on the VPS BEFORE the first
  `deploy.sh` with the new layout, or prod comes up with an EMPTY database.
- `compose.dev.yaml` + `Dockerfile.dev` (dev-only overlay; content VERIFIED
  WORKING 2026-08-18 as `compose.override.yaml`, restructured 2026-08-21 —
  re-verify the merged config on first use): builds the dev image (rust:1-
  slim-bookworm + cargo-watch BAKED IN via `cargo install --locked`, plus a
  uid-1000-owned /cargo-home — the runtime user is non-root, so no apt at
  startup and CARGO_HOME must be writable), then runs
  `cargo watch --poll -w src -w Cargo.toml -x "run --release"` inside the
  container against the repo mount. `--poll` is what makes it work on
  Docker Desktop: cargo-watch stats mtimes through virtiofsd (coherent)
  instead of relying on inotify (never delivered). `-w src -w Cargo.toml`
  is REQUIRED: default watching covers the whole repo, and the server
  rewrites its own state files (authorized_keys.yml on new-name logins,
  config, logs) — watching those would rebuild+restart on every login
  (verified: touching authorized_keys.yml + config does NOT restart with
  the restricted watch). First `up -d --build`: ~10 min image build
  (cargo install cargo-watch) + full dep compile into the target-cache
  volume; later restarts ≈ 20 s (deps cached), src-only rebuilds ≈ 1 min.
  Cosmetic: the container flaps "unhealthy" while a rebuild runs (server
  not listening yet) — docker only reports, it does NOT restart the
  container (no autoheal).
  History: the previous override ran `apt-get install cargo-watch` as the
  non-root user at startup → "Permission denied" crash loop (exit 100),
  and cargo-watch isn't in Debian repos anyway.
