# AGENTS.md — XavierDB

XavierDB: a Rust/axum HTTP server exposing MongoDB through a REST API — JWT
auth, granular permissions (`authorized_keys.yml`), adaptive per-app document
limits, binary config with undo/redo, embedded admin dashboard.

This file holds only the minimum standing instructions. Reference facts live
in `.agents/knowledge/`, procedural how-tos in `.agents/skills/` (indexes at
the bottom). Each fact has exactly ONE canonical home — read the relevant
file(s) before starting work, update them after.

## 0. Agent rules (read first)

1. **Python: always `uv run python` (or `uv run --with <pkg> python -c "..."`),
   never a system `python`.** A system interpreter may be missing, be an
   unusable stub, or be the wrong version — `uv` fetches a managed interpreter
   on demand (verified: Python 3.14 via uv 0.11.x). If `uv` is absent, install
   it with the official installer (see `.agents/knowledge/toolchain.md` for
   detection + install). Example: `uv run --with pyyaml python -c "import yaml"`.
2. **Never `git commit`.** The user handles all commits (their GPG signing is
   misconfigured; commits fail anyway). Make changes in the working tree only.
3. **Docker is optional.** If `docker` and the compose plugin are not on PATH
   (`command -v docker && docker --version && docker compose version`), run
   bare metal (`.agents/skills/build-run-test.md`). The compose/Docker setup
   (`.agents/skills/docker.md`) is VERIFIED on Docker Desktop (2026-08-16)
   with one known limitation: file-watcher hot reloads don't work in
   containers on Docker Desktop (inotify over VirtioFS) — see docker.md.
4. **Some OSes refuse to overwrite a running executable** — `cargo build`
   fails until the server is killed (on the dev machine the error is
   "Accès refusé"). Restart ritual (each step a SEPARATE shell command; the
   start must be its own command — a shell that times out can kill the
   disowned server: keep commands short and use `--max-time` on curls):
   1. stop the server by process name — POSIX: `pkill XavierDB`; Windows:
      `taskkill //F //IM XavierDB.exe` (plain `taskkill /F /IM XavierDB.exe`
      in non-MSYS shells). Verify down (`curl /health` fails).
   2. `cargo build --tests` (rebuilds the server binary AND keeps the
      test-mode fingerprints fresh)
   3. start detached (own command), e.g.
      `./target/debug/XavierDB >> /tmp/xdb.log 2>&1 & disown`
      (the binary is `XavierDB.exe` on Windows).
   **Trap:** never run plain `cargo build` between `cargo build --tests` and
   `cargo test` — the normal-mode build re-invalidates the test-mode bin
   fingerprint (mongodb is built as two separate units, one per graph) and
   `cargo test` then tries to relink the server binary → lock. `cargo test
   --no-run` has the same effect. The only clean sequence is:
   kill → `cargo build --tests` → start → `cargo test`. `cargo check` is safe
   while the server runs. Full details: `.agents/skills/restart-ritual.md`.
5. **Keep the `.agents/` knowledge current.** Before starting work, read the
   relevant `.agents/knowledge/` file(s) and follow the `.agents/skills/`
   how-tos; after work, update them so they stay current and standalone.
   Keep `.agents/` machine-agnostic: per-OS command variants (POSIX /
   Windows) only where commands genuinely differ; machine-local facts belong
   in `.pi/notes/credentials.md`.
6. `server.yml` (settings, contains `admin.password_hash`) may be awkward to
   touch from some shells (a protected path on the dev machine) — read it via
   `read`/`cat` with care; `$` in the hash needs no quoting in YAML. `.env`
   now holds only `UID`/`GID` (Docker-compose interpolation — the app never
   reads it).
7. Credentials are machine-local: read them from `.pi/notes/credentials.md`
   (gitignored — never commit or copy them into repo files). See
   `.agents/skills/credentials.md` for how to obtain or regenerate them on a
   fresh machine.

## Build / run / test

```bash
npm install && npm run build     # rebuild dashboard TS -> src/assets/app.js (only if TS changed)
(cd web && npm install && npm run docs:build)  # rebuild VitePress site (web/ is its own npm project)
# typecheck: npx --yes -p typescript tsc --noEmit --strict --target es2020 --lib es2020,dom src/assets/ts/app.ts
cargo build                      # fails while the server is running on some OSes — see rule 4
cargo test                       # 50 unit + 110 integration; needs a running server + MongoDB
./target/debug/XavierDB          # from repo root; cwd-relative state files; no CLI args
                                 # (settings: server.yml + env overrides; no .env)
```

Any rebuild that touches the server binary follows the kill →
`cargo build --tests` → start → `cargo test` ritual (rule 4). Dashboard asset
changes need `npm run build` + the same ritual (compile-time embed).
Full pipeline + integration battery: `.agents/skills/build-run-test.md`;
dashboard specifics: `.agents/skills/dashboard-rebuild.md`.

## Knowledge index (.agents/knowledge/)

| file | contents |
|---|---|
| `overview.md` | what this is, route table |
| `repo-layout.md` | full file tree, Docker image vs. repo mechanics |
| `config-world.md` | runtime state files (`server.yml`/`.env`/`config`/`authorized_keys.yml`/logs), hot reload, watchers |
| `architecture.md` | auth, perms, `/q` proxy, `/ls`, adaptive limit, config file, health, TLS, dashboard + UI architecture |
| `api.md` | client + dashboard API contracts |
| `docs-index.md` | what each doc file covers |
| `toolchain.md` | tool discovery, required versions |
| `known-limits.md` | by-design limits, known gaps, open items, deferred work |

## Skills index (.agents/skills/)

| file | contents |
|---|---|
| `restart-ritual.md` | kill/rebuild/restart ritual + the `cargo build --tests` fingerprint trap |
| `build-run-test.md` | full build pipeline, examples, integration battery |
| `dashboard-rebuild.md` | dashboard asset rebuild cycle + jsdom harness pattern |
| `examples.md` | examples/ crate commands + verified server facts |
| `docker.md` | compose/Docker deployment ops (VERIFIED 2026-08-16 on Docker Desktop; watcher limitation — see rule 3) |
| `credentials.md` | credential regeneration recipes |
| `perms-watcher-ritual.md` | perms/config watcher snapshot & restore ritual |
