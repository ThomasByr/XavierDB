# AGENTS.md — XavierDB

XavierDB: a Rust/axum HTTP server exposing MongoDB through a REST API — JWT
auth, granular permissions (`authorized_keys.yml`), adaptive per-app document
limits, binary config with undo/redo, embedded admin dashboard.

This file holds only the minimum standing instructions. Reference facts live in
`.agents/knowledge/`, procedural how-tos in `.agents/skills/<name>/SKILL.md`
(each with a runnable script in the same folder), role briefs in
`.agents/personas/`, script defaults in `.agents/settings/`.
**The full index: `.agents/INDEX.md`** — read the relevant file(s) there
before starting work and update the index after changing the tree.

## 0. Agent rules (read first)

1. **Python: always `uv run python` (or `uv run --with <pkg> python -c "..."`),
   never a system `python`.** A system interpreter may be missing, be an
   unusable stub, or be the wrong version — `uv` fetches a managed interpreter
   on demand (verified: Python 3.14 via uv 0.11.x). If `uv` is absent, install
   it with the official installer (see `.agents/knowledge/toolchain.md` for
   detection + install).
2. **Never `git commit`.** The user handles all commits (their GPG signing is
   misconfigured; commits fail anyway). Make changes in the working tree only.
3. **TESTS ALWAYS RUN AGAINST THE DOCKER STACK — Docker is the default for
   running, building, deploying AND testing.** `docker` + the compose plugin
   must be on PATH (`command -v docker && docker --version && docker compose
   version`). Default flow: `xdb-compose.sh up` (API + MongoDB in containers)
   → `battery.sh run` (local `cargo test` against the published ports) → the
   dashboard jsdom harnesses (`xdb-dashboard.sh harness <name>.mjs`). Compose
   details: `.agents/skills/docker/SKILL.md`.
   **MongoDB is ALWAYS launched with Docker** — the compose `mongodb` service,
   or a standalone `docker run -p 27017:27017 mongo:8.0` container even when
   the *server* runs on bare metal. Never start a bare-metal `mongod` on the
   toolchain path (`.pi/notes/credentials.md` holds a last-resort portable
   mongod note only).
4. **Bare metal is an EXCEPTION path, only when Docker fails** — e.g. Docker
   Desktop filesystem quirks (inotify is never delivered over VirtioFS bind
   mounts, so `watcher_reload` fails against the containerized API), a broken
   daemon/image build, or low-level host Rust checks. Then follow
   `.agents/skills/docker-fallback/SKILL.md` and its `xdb-restart.sh`
   `kill|build|start|test` subcommands — each a SEPARATE shell command (`start`
   must stay its own command: a shell that times out can kill the disowned
   server). The `cargo build --tests` fingerprint trap documented there applies
   to that path ONLY — inside Docker the image build is isolated and nothing
   locks.
5. **Keep the `.agents/` knowledge current.** Before starting work, read the
   relevant `.agents/knowledge/` file(s) and follow the `.agents/skills/`
   how-tos; after work, update them so they stay current and standalone
   (update `.agents/INDEX.md` when the tree changes).
6. `server.yml` (settings, contains `admin.password_hash`) may be awkward to
   touch from some shells (a protected path on the dev machine) — read it via
   `read`/`cat` with care; `$` in the hash needs no quoting in YAML. `.env`
   now holds only `UID`/`GID` (Docker-compose interpolation — the app never
   reads it).
7. Credentials are machine-local: read them from `.pi/notes/credentials.md`
   (gitignored — never commit or copy them into repo files). See
   `.agents/skills/credentials/SKILL.md` for how to obtain or regenerate them
   on a fresh machine.

## Build / run / test — use the scripts

All commands behind these are documented in `.agents/INDEX.md` + the SKILL.md
of each folder; prefer a script over a hand-typed command:

- Docker stack (API + MongoDB; the DEFAULT): `.agents/skills/docker/xdb-compose.sh`
  (`up` / `watch` / `build` / `logs` / `restart` / `ps` / `down` / `deploy` / `battery`)
- integration battery (fixture bootstrap + `cargo test`):
  `.agents/skills/build-run-test/battery.sh`
- dashboard rebuild + jsdom harnesses:
  `.agents/skills/dashboard-rebuild/xdb-dashboard.sh`
- examples crate: `.agents/skills/examples/examples.sh`
- bare-metal fallback (docker issues only):
  `.agents/skills/docker-fallback/xdb-restart.sh`

Startup manual (fallback hand-type only): the server binary is
`./target/debug/XavierDB` (`XavierDB.exe` on Windows), run from the repo root;
cwd-relative state files, no CLI args (settings: `server.yml` + env overrides;
no `.env`).