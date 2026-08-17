# Docker / compose deployment — VERIFIED 2026-08-16 on Docker Desktop 29.7.2 (WSL2)

First real run: `docker compose up --build -d` worked end to end on the dev
machine (Docker Desktop 29.7.2, WSL2 backend, overlayfs). The integration
battery (local rust, `cargo test`) against the docker stack is 108/110 —
the 2 watcher_reload failures are a Docker-Desktop-only limitation, see
"inotify / file watchers" below.

## Ops commands

```bash
docker compose up --build -d     # builds API image + starts MongoDB + API (incremental: layer cache)
docker compose watch             # rebuilds image on ./Cargo.toml or ./src changes
docker compose build --no-cache api   # force a full rebuild when the cache is suspect
docker compose logs api          # first-run dashboard password is printed here
docker compose restart api       # needed after server.yml changes (read at process start only)
```

## compose.yaml facts (verified)

- api has **no `image:` key** (build-only; with both, compose would tag the
  build and clobber the official `rust:1-slim-bookworm` tag).
- Mongo healthcheck (marker-file hack, retries 100 × 5s) and the api
  healthcheck (curl /health, https fallback) both verified green.
- **Env precedence (since 2026-08-16): env var > `server.yml` > default.**
  The app no longer reads `.env` at all — all startup settings live in
  `server.yml` (YAML, startup-only, no hot reload), shared by bare metal and
  Docker through the repo mount. Compose's `HOST=0.0.0.0` and
  `MONGODB_URI=mongodb://xavierdb:27017` env vars OVERRIDE the file, so the
  same server.yml with bare-metal defaults (127.0.0.1 / localhost) works in
  both worlds. EXCEPTION: `admin.username`/`admin.password_hash` always come
  from the file (Windows always sets `USERNAME` — an env override would
  silently break the dashboard login on bare metal). `.env` now holds ONLY
  `UID`/`GID` (compose `user:` interpolation).
- `user: ":"` (mongo) and `user: "$UID:$GID"` (api, from .env UID/GID=1000)
  both run fine on Docker Desktop; the repo mount stays transparently
  editable from Windows.
- Mongo volume: intended `${HOME}/data/xavier-mongo-db` bind mount. On this
  Windows host `${HOME}` is undefined, so compose.yaml TEMPORARILY uses the
  named volume `xavier_mongo_db` instead (marked in the file; revert to the
  bind mount on a Linux host).
- First boot with no `server.yml` in the repo: the server creates it from the
  embedded `server.yml.example` template and (blank `admin.password_hash`)
  generates a dashboard password printed once — `docker compose logs api`.

## inotify / file watchers (the one real limitation, verified)

The perms/config/TLS hot reloads are **pure inotify** (`watch_file` in
src/main.rs, no polling fallback). Over a Docker Desktop bind mount
(VirtioFS) **no inotify events are delivered at all** — verified: even a
write done INSIDE the container to a freshly-watched file never fires the
watcher (virtiofsd implements no FUSE notify). Consequences:

- `cargo test` against a Docker-Desktop API: `watcher_reload` fails
  (`perms_file_watcher_reload` at the "watcher picked up the appended app"
  assert; `reload_endpoints` then dies on the poisoned suite lock — it passes
  standalone). Everything else is green (108/110).
- On bare metal (Linux/Windows host kernel inotify) and on real Linux Docker
  hosts (kernel bind mount) the watchers work — the battery was green there
  before.
- Dashboard/perms writes still WORK in docker (the API reloads its own
  writes directly); only external file edits go unnoticed. Manual fix:
  `docker compose restart api`, or use the `/perms/reload` +
  `/config/reload` endpoints (relay works in docker).

## Dockerfile facts (verified)

- Stage 1 node:22-bookworm-slim: `npm ci` (lockfile), `npm run build`
  (esbuild ts/app.ts → src/assets/app.js — the only generated asset;
  index.html/styles.css are static, come from the context).
- Stage 2 rust:1-slim-bookworm: apt install cmake perl pkg-config
  (aws-lc-sys needs them; gcc+libc6-dev already in the image) + curl
  ca-certificates (healthcheck); dummy-main layer-cache trick; `COPY . .`
  then overlay app.js from the node stage; binary → /usr/local/bin/XavierDB;
  WORKDIR /app; CMD ["XavierDB"]. Full build incl. aws-lc-sys verified
  (~4 min cold on the dev machine).
- Single-stage (runtime = rust image, ~1.5–2 GB) per user constraint.
- include_str! needs src/assets/{index.html,styles.css,app.js} + server.yml.example
  in the build context at compile time.
- Image NEVER contains state files (.dockerignore + COPY lands in image
  root /; /app is empty in the image) — see knowledge/repo-layout.md.

## Build speed — findings & pending fixes (2026-08-17, verified, NOT applied)

Measured during the 2026-08-17 build-speed session (Docker Desktop 29.7.2).
All three were reverted per user decision (commit happened with the
original Dockerfile); re-apply when build speed work resumes.

1. **Build context is ~1.6 GB, most of it leaked `examples/target`**
   (1.46 GB). `.dockerignore`'s bare `target` pattern only matches the ROOT
   directory — nested ones need `**/target`. `web/.vitepress/dist` +
   `web/.vitepress/cache` (~20 MB) leaked too. Measured with exclusions:
   context transfer 1.63 GB/103 s → 4.6 kB/0 s on every build.
   Fix (safe): add `**/target`, `**/node_modules`, `web/.vitepress/dist`,
   `web/.vitepress/cache` to `.dockerignore`.

2. **Dependencies are compiled TWICE per build.** The dummy-main layer does
   `COPY Cargo.toml ./` WITHOUT `Cargo.lock`, so it resolves fresh "latest"
   versions; the real build then sees the repo's `Cargo.lock` (different
   versions) and recompiles ~30 crates from scratch — including aws-lc-sys
   (~35 s). With `COPY Cargo.toml Cargo.lock ./` the dummy build compiles the
   exact locked versions and the real step only compiles the app crate
   (~10 s instead of ~100 s). Fix (safe): copy the lockfile in the dummy
   layer.

3. **PITFALL — never `--mount=type=cache,target=/build/target` on the build
   steps.** A target-dir cache mount SHADOWS the dummy-main layer's compiled
   deps and shares state between the dummy and real steps; the dummy step
   leaves a ~400 KB `fn main(){}` binary in the cache and cargo can consider
   the bin fresh in the real step → `cp target/release/XavierDB` copies the
   DUMMY into the image → the container exits 0 silently (crash loop, no
   logs; observed live: image binary 437 KB). A cache mount for
   `/usr/local/cargo/registry` ONLY (downloads) is safe and helps cold
   rebuilds.

Baseline for comparison: no-op rebuild ~2 s (all layers cached), src-only
rebuild with the current (reverted) Dockerfile ≈ 2–4 min (mostly context
transfer + the double compile from #1/#2).
