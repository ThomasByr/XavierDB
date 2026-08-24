#!/usr/bin/env bash
# .agents/skills/perms-watcher-ritual/perms-watcher.sh
#
# Snapshot / restore the watcher-managed state files
# (authorized_keys.yml and the binary `config`) for testing permission /
# config changes + hot reload verification.
#
# Usage:
#   perms-watcher.sh snapshot <file> [label]    copy <file> to $XDB_SNAPSHOT_DIR
#   perms-watcher.sh restore <file>             restore the LATEST snapshot of that file
#   perms-watcher.sh restore <file> <snapshot>  restore a specific snapshot (path or name)
#   perms-watcher.sh list [<file>]              show snapshots
#
# Ritual (see SKILL.md):
#   1. snapshot   cp the live file away
#   2. change     hand-edit or drive the dashboard (watcher reloads in ~500 ms)
#   3. verify     check the API reflects the change (GET /dashboard/api/perms
#                 or /config, or behaviour of /q/)
#   4. restore    copy the snapshot back — a byte-identical restore IS picked
#                 up automatically (re-stamp fix 2026-08-14), give the watcher
#                 its debounce window, then re-verify.
#
# Traps (SKILL.md): hand-edit while the server is IDLE (a self-write within the
# ~500 ms debounce can silently swallow an external edit); the watcher cannot
# attach to a file that didn't exist at startup (restart the server); atomic-
# rename editors may detach the notify watch.
#
# Overridable env (see .agents/settings/defaults.sh): XDB_REPO, XDB_SNAPSHOT_DIR.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${XDB_REPO:=$(cd "$SCRIPT_DIR/../../.." && pwd)}"
# shellcheck disable=SC1091
. "$XDB_REPO/.agents/settings/defaults.sh"
XDB_REPO="$(cd "$XDB_REPO" && pwd)"

SNAP_DIR="$XDB_SNAPSHOT_DIR"
die() { echo "ERROR: $*" >&2; exit 1; }

do_snapshot() {
  local file="${1:-}" label="${2:-}" target ts
  [ -n "$file" ] || die "snapshot needs a file (e.g. authorized_keys.yml or config)"
  [ -f "$file" ] || die "no such file: $file"
  mkdir -p "$SNAP_DIR"
  ts="$(date +%Y%m%d-%H%M%S)"
  if [ -n "$label" ]; then
    target="$SNAP_DIR/$(basename "$file").$ts.$label.bak"
  else
    target="$SNAP_DIR/$(basename "$file").$ts.bak"
  fi
  cp "$file" "$target"
  echo "Snapshot: $target"
}

do_list() {
  local pat="${1:-*.bak}"
  [ -d "$SNAP_DIR" ] || { echo "no snapshots yet ($SNAP_DIR)"; return 0; }
  if [ -n "${1:-}" ]; then
    (cd "$SNAP_DIR" && ls -1t "$(basename "$1")".*.bak 2>/dev/null | sed 's/^/  /')
  else
    (cd "$SNAP_DIR" && ls -1t ./*.bak 2>/dev/null | sed 's/^/  /')
  fi
}

do_restore() {
  local file="${1:-}" snap="${2:-}" src
  [ -n "$file" ] || die "restore needs a file (e.g. authorized_keys.yml or config)"
  [ -d "$SNAP_DIR" ] || die "no snapshots dir: $SNAP_DIR (snapshot first)"
  if [ -n "$snap" ]; then
    if [ -f "$snap" ]; then src="$snap"
    elif [ -f "$SNAP_DIR/$snap" ]; then src="$SNAP_DIR/$snap"
    else die "no such snapshot: $snap (see 'list')"
    fi
  else
    src="$(cd "$SNAP_DIR" && ls -t "$(basename "$file")".*.bak 2>/dev/null | head -n 1 || true)"
    [ -n "$src" ] || die "no snapshots for $(basename "$file") — run 'snapshot $file' first"
    src="$SNAP_DIR/$src"
  fi
  cp "$src" "$file"
  echo "Restored $file <- $src"
  echo "The watcher picks up a byte-identical restore automatically (500 ms debounce) — wait, then verify."
}

usage() { sed -n '2,24p' "$0" | sed 's/^# \{0,1\}//'; }

cmd="${1:-help}"; [ $# -gt 0 ] && shift
case "$cmd" in
  snapshot) do_snapshot "$@" ;;
  restore)  do_restore "$@" ;;
  list)     do_list "${1:-}" ;;
  help|-h|--help) usage ;;
  *) die "unknown subcommand: $cmd — run '$0 help'" ;;
esac