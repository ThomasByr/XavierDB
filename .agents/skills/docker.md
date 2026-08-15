# Docker / compose deployment — UNVERIFIED, never run anywhere yet

Development happened without Docker. All of the following is UNVERIFIED:
full build (aws-lc-sys in container, cmake/perl/pkg-config install),
healthcheck behavior, `${HOME}` interpolation on the user's Docker host,
notify-watcher behavior inside a container. Statically reviewed only.

## Ops commands

```bash
docker compose up --build -d     # builds API image + starts MongoDB + API (incremental: layer cache)
docker compose watch             # rebuilds image on ./Cargo.toml or ./src changes
docker compose build --no-cache api   # force a full rebuild when the cache is suspect
docker compose logs api          # first-run dashboard password is printed here
docker compose restart api       # needed after .env changes (dotenvy reads at process start only)
```

## compose.yaml facts

- api has **no `image:` key** (build-only; with both, compose would tag the
  build and clobber the official `rust:1-slim-bookworm` tag).
- Mongo healthcheck uses a marker-file hack (retries 100 × 5s); api
  healthcheck is plain `curl -fsS http://localhost:8000/health`.
- Compose `environment:` (`HOST=0.0.0.0`, `MONGODB_URI=mongodb://xavierdb:27017`)
  wins over `.env` (dotenvy never overrides existing env vars).
- `develop.watch` rebuilds on source change; the repo mount does NOT
  hot-reload Rust code (binary lives at `/usr/local/bin/XavierDB`, outside
  `/app`).
- Mongo volume: `${HOME}/data/xavier-mongo-db` (binary DB data stays out of
  the repo).

## Caveats

- On a LINUX Docker host, container writes to the repo (default config
  creation, config.bak rotation, `.env` bootstrap) are root-owned — you may
  need sudo to edit them; Desktop-style mounts (Docker Desktop) are
  transparent. Fix if it bites: `user: "${UID}:${GID}"` (Linux hosts only).
- notify watcher over a bind mount + atomic-rename editors (vim etc.): the
  watch may detach on inode replace — if hot reload stops after an editor
  save, `docker compose restart api`.

## Dockerfile facts

- Stage 1 node:22-bookworm-slim: `npm ci` (lockfile), `npm run build`
  (esbuild ts/app.ts → src/assets/app.js — the only generated asset;
  index.html/styles.css are static, come from the context).
- Stage 2 rust:1-slim-bookworm: apt install cmake perl pkg-config
  (aws-lc-sys needs them; gcc+libc6-dev already in the image) + curl
  ca-certificates (healthcheck); dummy-main layer-cache trick; `COPY . .`
  then overlay app.js from the node stage; binary → /usr/local/bin/XavierDB;
  WORKDIR /app; CMD ["XavierDB"].
- Single-stage (runtime = rust image, ~1.5–2 GB) per user constraint.
- include_str! needs src/assets/{index.html,styles.css,app.js} + .env.example
  in the build context at compile time.
- Image NEVER contains state files (.dockerignore + COPY lands in image
  root /; /app is empty in the image) — see knowledge/repo-layout.md.
