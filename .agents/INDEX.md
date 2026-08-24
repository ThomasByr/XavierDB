# .agents/INDEX.md — the single index

The full index for the `.agents/` tree. **Read the relevant file(s) before
starting work; update this index after adding/removing files.** Each fact has
exactly ONE canonical home: reference facts live in `.agents/knowledge/`,
procedural how-tos in `.agents/skills/<name>/SKILL.md` — each with a runnable
script in the same folder — role briefs in `.agents/personas/`, and script
defaults in `.agents/settings/`. Machine-local facts stay in
`.pi/notes/credentials.md` (gitignored).

## Knowledge (.agents/knowledge/)

| file | contents |
|---|---|
| `overview.md` | what this is, route table |
| `repo-layout.md` | full file tree, Docker image vs. repo mechanics, `.agents/` tree |
| `config-world.md` | runtime state files (`server.yml`/`.env`/`config`/`authorized_keys.yml`/logs), hot reload, watchers |
| `toolchain.md` | tool discovery, required versions, install-if-missing |
| `docs-index.md` | what each doc file covers + the docs-drift standing rule |
| `api.md` | client + dashboard API contracts |
| `known-limits.md` | by-design limits, known gaps, open items, deferred work, verification checkpoints |

## Knowledge — architecture (.agents/knowledge/architecture/)

Split from the former `architecture.md` (2026-08-24); the section map lives in
`architecture/README.md`.

| file | contents |
|---|---|
| `README.md` | section map + editing rules for the architecture files |
| `auth.md` | `/auth`, JWT, Argon2id + throttles, auth Q&A |
| `perms.md` | `authorized_keys.yml` structure, layered first-match-wins, `/indexes` perm model |
| `proxy.md` | `/q` verbs, keyset cursor pagination, filter hardening, index endpoints, projection, batch-write driver facts |
| `ls.md` | `GET /ls` contract, listing-cursor rule |
| `adaptive-limit.md` | adaptive per-app limit formula, container-aware system sampling |
| `config-file.md` | binary config: magic/history/undo, sanitize clamps, key fields |
| `health.md` | `/health` shape + verified behavior |
| `tls.md` | optional TLS, cert/key hot reload |
| `dashboard.md` | SPA architecture per tab + request log line formats |

## Skills (.agents/skills/<name>/)

Each skill = `SKILL.md` (the how-to) + a script that captures the embedded
commands, runnable and CLI-configurable (defaults overridable via `XDB_*` env —
see `settings/`).

| skill | doc | script |
|---|---|---|
| `docker-fallback/` | **exception path only** — bare-metal server when Docker fails (filesystem quirks, broken daemon, host Rust checks); MongoDB stays in Docker | `xdb-restart.sh` — `kill / kill-port <port> / build / start / test [<area>] / cycle / status` |
| `build-run-test/` | Docker-first build/test pipeline (battery vs the stack), examples, fallback notes | `build.sh` (`bundle / typecheck / server [--release] / all`) + `battery.sh` (`bootstrap / run / single <area> / all`) |
| `dashboard-rebuild/` | dashboard asset rebuild cycle + jsdom harness pattern | `xdb-dashboard.sh` — `bundle / typecheck / harness <name.mjs> / harnesses` |
| `examples/` | examples/ crate commands + verified server facts | `examples.sh` — `build / list / run <bin> [-- args…]` |
| `docker/` | compose/Docker deployment ops — the DEFAULT stack (API + MongoDB in containers; MongoDB is ALWAYS Docker) | `xdb-compose.sh` — `up / watch / build [--no-cache] / logs / password / restart / ps / down / deploy / battery / mongo` |
| `credentials/` | credential regeneration recipes (machine-local secrets live in `.pi/notes/credentials.md`) | `hash-token.py` (Argon2id via uv) + `gen-cert.sh` (self-signed TLS) |
| `perms-watcher-ritual/` | perms/config watcher snapshot & restore ritual | `perms-watcher.sh` — `snapshot <file> [label] / restore <file> [snapshot] / list` |

## Personas (.agents/personas/)

Role briefs for focused sub-agents; each states context, conventions, and
verification before "done". See `personas/README.md` (note: these are repo
conventions, not pi agent types).

| file | use for |
|---|---|
| `rust-backend.md` | any change to `src/` (server, routes, auth, perms, config, metrics, tls) |
| `dashboard-ts.md` | any change to `src/assets/ts/` (dashboard SPA) + jsdom harnesses |
| `docs-writer.md` | any change to `docs/`, `README.md`, or `.agents/knowledge/` |
| `security-reviewer.md` | security review of auth/perms/throttling/input handling |

## Settings (.agents/settings/)

| file | contents |
|---|---|
| `README.md` | what settings are (config, not facts) + conventions |
| `defaults.sh` | bash-sourced shared defaults for every skills script (repo root, bin, port, Mongo URI, log path, snapshot dir, dashboard creds) — all `XDB_*`, env-overridable. **The server never reads this file.** |

## Editing rules

1. Keep the `.agents/` tree machine-agnostic: per-OS command variants only
   where commands genuinely differ; machine-local facts belong in
   `.pi/notes/credentials.md`.
2. Each fact has ONE canonical home — update the relevant file, not a copy.
3. After any change that adds/removes/moves a file, update this index and
   `knowledge/repo-layout.md` in the same pass.
4. After a change that touches routes, permissions/actions, throttling, config
   fields/defaults/clamps, or the dashboard UI, re-check the user-facing docs
   (`docs/`) too — see `knowledge/docs-index.md`.