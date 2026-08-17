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
| MongoDB (mongod) | runtime + integration battery | `command -v mongod && mongod --version` | official MongoDB packages; **8.0+ required** |
| Docker + compose plugin | optional (compose deployment) | `command -v docker && docker --version && docker compose version` | docker.com; WITHOUT Docker, run bare metal (see skills/build-run-test.md) |

Required versions / facts:
- **MongoDB 8.0+** — PATCH upsert-many uses the driver's `bulk_write` (new
  bulkWrite command, no legacy path). The dev machine runs 8.0.12.
- **Rust**: edition 2024 → rustup latest stable is fine (developed on 1.97).
- **Node**: only at build time (esbuild 0.28); any modern LTS is fine.
  **Python**: only dev scripts; always `uv run python`. **mongosh** is
  optional — shell access only; neither the server nor the battery needs it.
- A bare-metal run needs only two things: `mongod` reachable at
  `mongodb://localhost:27017` (override with `network.mongodb_uri` in
  `server.yml`, or the `MONGODB_URI` env var) and
  `./target/debug/XavierDB` started from the repo root (cwd-relative state
  files, no CLI args).
- Machine-local facts (portable-mongod layout, shell quirks, ops commands)
  live in `.pi/notes/credentials.md` — never in repo files.

Credential regeneration recipes: see `.agents/skills/credentials.md`.
