---
name: perms-watcher-ritual
description: Snapshot & restore ritual for authorized_keys.yml and the binary config file via perms-watcher.sh, to test permission/config changes and verify hot reload (notify watcher, ~500 ms debounce). Includes the loss-window, non-existent-file, and atomic-rename watcher traps. Use when editing perms/config and checking watcher reload.
---

# Perms/config watcher — snapshot & restore ritual

> **Script:** `perms-watcher.sh` (same dir) — `snapshot <file> [label] |
> restore <file> [snapshot] | list [<file>]`. Prefer it over hand-typed
> `cp` commands; snapshot dir + defaults overridable via `XDB_*` env (see
> `.agents/settings/defaults.sh`).

For testing permission or config changes and verifying hot reload (notify
watcher, 500 ms debounce):

1. **Snapshot**: copy the live file to a backup (e.g. `cp authorized_keys.yml
   /tmp/ak.bak`).
2. **Change**: hand-edit the file, or drive the dashboard (perms editor /
   config tab). The watcher reloads within ~500 ms; a successful reload
   re-stamps the loaded bytes.
3. **Verify**: check the API reflects the change (e.g. `/q/` behaves per the
   new rules, or `GET /dashboard/api/perms` / `GET /dashboard/api/config`
   shows the new state).
4. **Restore**: copy the backup back over the file. **A byte-identical
   restore IS picked up automatically** (re-stamp fix, 2026-08-14) — no
   explicit `/perms/reload` needed. Give the watcher its debounce window,
   then verify state is back.

## Traps

- **Loss window**: an external hand-edit can be silently lost if the server
  writes its own copy within the ~500 ms debounce (the self-write byte-stamp
  then suppresses the reload). Hand-edit while the server is idle, or use the
  dashboard.
- **Non-existent file at startup**: the watcher cannot attach to a file that
  doesn't exist ("file may not exist yet") — if you create
  `authorized_keys.yml` after startup, restart the server.
- **Atomic-rename editors** (vim etc.) may detach the notify watch — if hot
  reload stops after an editor save, restart re-attaches.
- Invalid files → previous state kept + error logged (no crash).
- This applies to BOTH `authorized_keys.yml` and the binary `config` file.
