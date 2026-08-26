---
name: docker-fallback
description: Bare-metal XavierDB server fallback ritual when Docker cannot run — xdb-restart.sh kill|build|start|test subcommands plus the cargo build --tests vs cargo test clean-sequence trap. ACTIVATE ONLY when the Docker default (xdb-compose.sh up) is broken; MongoDB always stays in Docker even here.
---

# Docker fallback — bare-metal server when Docker has issues

> **Script:** `xdb-restart.sh` (same dir) — `kill | kill-port <port> | build |
> start | test [<area>] | cycle | status` as SEPARATE subcommands. Prefer it over
> hand-typed commands; each step stays its own shell invocation (the `start`
> trap below), defaults overridable via `XDB_*` env (see
> `.agents/settings/defaults.sh`).

**ACTIVATE ONLY when the Docker default cannot run.** The DEFAULT is always:
`xdb-compose.sh up` (API + MongoDB in containers), then `battery.sh run`
(local cargo test against the published ports) — see `skills/docker/SKILL.md`
and `skills/build-run-test/SKILL.md`. This skill is the exception path.

## When to activate (check these before going bare metal)

- `docker` or the compose plugin are missing/broken
  (`command -v docker && docker --version && docker compose version`) — but
  first try to fix Docker; bare metal is a workaround, not a config.
- Docker Desktop filesystem quirks: **inotify is NEVER delivered over VirtioFS
  bind mounts**, so `watcher_reload` fails against the containerized API;
  other odd mounts / performance problems on Windows.
- The API container won't start or the image build fails in a way you can't
  fix quickly.
- You need low-level Rust checks ON the host (unit tests, `cargo check`,
  local debugging of the server process itself).

## MongoDB is ALWAYS Docker — even in fallback mode

The fallback applies to the **XavierDB server binary**, never to MongoDB:

- `docker compose up -d mongodb` (the compose `mongodb` service), or
- `docker run -d --name xdb-mongo -p 27017:27017 mongo:8.0`

Both publish `mongodb://localhost:27017` — the default the server and the
battery use. **Never start a bare-metal mongod on the toolchain path.** (If
Docker itself is unusable, the machine-local portable-mongod notes in
`.pi/notes/credentials.md` are the absolute last resort.)

## The ritual (why it exists)

**Some OSes refuse to overwrite a running executable** — `cargo build` fails
until the server is killed (on the dev machine the error is "Accès refusé").

Each step a SEPARATE shell command; the start must be its own command — a
shell that times out can kill the disowned server: keep commands short and
use `--max-time` on curls. (`A && B &` backgrounds the whole chain — run
build and server-start as SEPARATE commands.)

1. `xdb-restart.sh kill` — stop the server by process name (POSIX/Windows
   variant inside the script). Verify down (`curl /health` fails).
2. `xdb-restart.sh build` — `cargo build --tests` (rebuilds the server binary
   AND keeps the test-mode fingerprints fresh).
3. `xdb-restart.sh start` — start detached (own command), waits for `/health`
   (log default: `$XDB_REPO/target/xdb.log`).
4. `xdb-restart.sh test` — `cargo test` (sets `XDB_TEST_MONGO_URI` so the
   env-gated Mongo-backed unit tests run too; `test <area>` = `--test <area>`).

## TRAP — the clean-sequence rule (single most load-bearing build fact)

**Never run plain `cargo build` between `cargo build --tests` and
`cargo test`.** mongodb is compiled as TWO separate units (normal graph vs
test graph, different rlib hashes); the normal-mode build re-invalidates the
test-mode bin fingerprint ("Dirty: info of dependency mongodb changed") and
`cargo test` then tries to relink the server binary → lock. `cargo test
--no-run` has the same effect. A full `cargo clean` does NOT cure it.

The only clean sequence is: **kill → `cargo build --tests` → start →
`cargo test`** (= `xdb-restart.sh cycle`). `cargo check` is safe while the
server runs. This trap exists ONLY on the bare-metal path — inside Docker the
image build is isolated, nothing locks.

## When src/ changed (full test cycle)

`cargo test` relinks the bin test harness whenever src/ or Cargo.toml
changed — same lock hazard. Ritual:

1. `xdb-restart.sh kill` → verify down
2. `xdb-restart.sh build` ← the ONLY build needed; leaves the runnable
   server binary in place
3. `xdb-restart.sh start` (own command)
4. `xdb-restart.sh test` (or `XDB_TEST_MONGO_URI=… xdb-restart.sh test`)

Incremental test runs (no src/ change) don't relink — `xdb-restart.sh test`
alone is fine.

## Second server instance

A second instance can run with env overrides (e.g. `PORT=8443`) sharing the
same cwd state files — fine for read-only testing; stop it by PID
(`xdb-restart.sh kill-port 8443` — `lsof -i :8443` on POSIX,
`netstat -ano` on Windows), never by process name (that kills every instance).