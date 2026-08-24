# Architecture — section map

`knowledge/architecture.md` was split into per-topic files (2026-08-24) so the
facts match the code layout. The route/proxy facts span `proxy.md` (+
`ls.md`); server internals in `auth/perms/config-file/adaptive-limit`; ops
surfaces in `health.md` / `tls.md`; the dashboard SPA in `dashboard.md`.

| file | contents |
|---|---|
| `auth.md` | `/auth`, JWT, Argon2id + throttles, auth Q&A (JSON↔BSON fidelity, name/app identity) |
| `perms.md` | `authorized_keys.yml` structure, layered first-match-wins, `/indexes` perm model |
| `proxy.md` | `/q/<db>/<coll>` verbs + cursor (keyset) pagination + filter hardening, index endpoints, projection impl map, batch-write driver facts (mongodb 3.8) |
| `ls.md` | `GET /ls` (flat dbs / `?db=` collections), listing-cursor contract |
| `adaptive-limit.md` | adaptive per-app limit formula, container-aware system sampling |
| `config-file.md` | binary config: XDB1 magic, history/undo, sanitize clamps, key fields |
| `health.md` | `/health` document shape, caching, verified failure/recovery behavior |
| `tls.md` | optional TLS, cert/key hot reload |
| `dashboard.md` | SPA architecture: UI per tab (overview RPS/focus/archive, clients, config, logs), theme tokens, request log line formats |

## Editing rules

- Keep each section file focused; when a change touches several areas update
  each file concerned (facts have ONE canonical home — don't duplicate).
- Section cross-references use the file names above (e.g. "see
  `adaptive-limit.md`"), never the old single-file anchors.