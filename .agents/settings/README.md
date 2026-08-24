# .agents/settings/ — agent + script settings

Holds machine-agnostic settings that agents and the `.agents/skills/*/` scripts
consume. This folder is for **configuration** (values/knobs), NOT reference
facts — facts stay in `.agents/knowledge/`, how-tos in `.agents/skills/`.

| file | purpose |
|---|---|
| `defaults.sh` | bash-sourced shared defaults for every `.agents/skills/*/` script (repo root, bin name, port, Mongo URI, log path, snapshot dir, dashboard creds). Each `XDB_*` value is overridable from the environment. **The server never reads this file.** |

## Conventions

- Scripts source `defaults.sh` via `$XDB_REPO/.agents/settings/defaults.sh`
  (bash scripts auto-detect `XDB_REPO` from their own location — they don't
  need the caller to set it).
- Machine-local values (ports, log paths, installed tools on a given box)
  belong in `.pi/notes/credentials.md` (gitignored), never here. This folder
  only holds portable defaults.
- Learn how config worlds are wired in `knowledge/config-world.md` before
  changing anything the server reads. `defaults.sh` is deliberately detached
  from the server's own `server.yml` / binary `config`.
