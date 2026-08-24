# Environment & toolchain discovery

The server is cross-platform Rust; Python and Node are build/dev-time only.
On a fresh machine, discover what's already installed (POSIX shell syntax —
MSYS-style shells like git-bash accept it; PowerShell/cmd need their own
equivalents):

| tool | when needed | detect | install if missing |
|---|---|---|---|
| Rust (cargo + rustc) | always | `command -v cargo && cargo --version` | rustup (https://rustup.rs); edition 2024 needs rustc ≥ 1.85 (developed on 1.97) |
| Node + npm | build-time only (esbuild compiles the dashboard TS) | `command -v node && node --version` | nodejs.org or a version manager (nvm/fnm/volta) |
| uv | dev scripts (managed Python on demand) | `command -v uv && uv --version` | official installer: https://docs.astral.sh/uv/ |
| python3 | dev scripts ONLY via uv — a system interpreter is not required | `command -v python3` (informational) | none — uv fetches managed interpreters |
| Docker + compose plugin | **REQUIRED — the DEFAULT for running + testing** | `command -v docker && docker --version && docker compose version` | docker.com; if broken → bare-metal fallback (skills/docker-fallback/SKILL.md), but prefer fixing Docker |
| MongoDB (mongod) | runtime + integration battery | **no host binary needed — it runs in a container**; verify with `docker ps` (compose `mongodb` service) or `xdb-compose.sh mongo` | **ALWAYS Docker**: compose `mongodb` service, or `docker run -p 27017:27017 mongo:8.0` — **8.0+ required**. Never a bare-metal `mongod` (last resort only: portable-mongod notes in `.pi/notes/credentials.md`) |

Required versions / facts:
- **MongoDB 8.0+** — PATCH upsert-many uses the driver's `bulk_write` (new
  bulkWrite command, no legacy path). Runs in Docker (`mongo:8.0`, the compose
  `mongodb` service). The dev machine runs 8.0.12.
- **Rust**: edition 2024 → rustup latest stable is fine (developed on 1.97).
- **Node**: only at build time (esbuild 0.28); any modern LTS is fine.
  **Python**: only dev scripts; always `uv run python`. **mongosh** is
  optional — shell access only; neither the server nor the battery needs it
  (use the container one: `xdb-compose.sh mongo`).
- DEFAULT testing needs only Docker: `xdb-compose.sh up` (API + MongoDB in
  containers) then `battery.sh run` — the ports compose publishes
  (127.0.0.1:8000 / mongodb://localhost:27017) match the battery defaults.
- The bare-metal FALLBACK (docker issues, see skills/docker-fallback/SKILL.md)
  needs: MongoDB still via Docker (`docker compose up -d mongodb` or a
  `docker run` mongo) reachable at `mongodb://localhost:27017`, and
  `./target/debug/XavierDB` started from the repo root (cwd-relative state
  files, no CLI args). If Docker itself is unusable, the machine-local
  portable-mongod notes in `.pi/notes/credentials.md` are the absolute last
  resort.
- Machine-local facts (portable-mongod layout, shell quirks, ops commands)
  live in `.pi/notes/credentials.md` — never in repo files.

Credential regeneration recipes: see `.agents/skills/credentials/SKILL.md`.
