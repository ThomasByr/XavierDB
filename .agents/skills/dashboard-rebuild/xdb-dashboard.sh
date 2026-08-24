#!/usr/bin/env bash
# .agents/skills/dashboard-rebuild/xdb-dashboard.sh
#
# Dashboard rebuild cycle helper (compile-time embed — 2 steps!).
# The SPA is include_str!-embedded AT COMPILE TIME: ANY asset change (TS/CSS/
# HTML) needs the bundle + a server rebuild (xdb-restart.sh build/start).
#
# Usage:
#   xdb-dashboard.sh bundle      npm install (if needed) + npm run build (esbuild)
#   xdb-dashboard.sh typecheck   tsc --noEmit on the TS (esbuild does NOT typecheck)
#   xdb-dashboard.sh all         bundle + typecheck                  (default)
#   xdb-dashboard.sh harness <n> run node tests/dashboard/<n>.mjs    (jsdom repro)
#   xdb-dashboard.sh harnesses   list the jsdom harnesses under tests/dashboard/
#
# After `bundle`, re-serve the new embed: DEFAULT = `xdb-compose.sh up`
# (rebuilds the image; the jsdom harnesses read src/assets/app.js — same bytes
# as the embed, but only after `bundle` regenerated it). FALLBACK (docker
# issues): the kill -> cargo build --tests -> start ritual in
# skills/docker-fallback/SKILL.md.
#
# Overridable env (see .agents/settings/defaults.sh): XDB_REPO.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${XDB_REPO:=$(cd "$SCRIPT_DIR/../../.." && pwd)}"
# shellcheck disable=SC1091
. "$XDB_REPO/.agents/settings/defaults.sh"
XDB_REPO="$(cd "$XDB_REPO" && pwd)"

cd "$XDB_REPO"
die() { echo "ERROR: $*" >&2; exit 1; }

do_bundle() {
  if [ ! -d node_modules ]; then
    echo "node_modules missing — running npm install"
    npm install
  fi
  echo "Running: npm run build (esbuild ts/app.ts -> src/assets/app.js)"
  npm run build
  echo "OK — bundle rebuilt. Re-serve the new embed:"
  echo "     DEFAULT: .agents/skills/docker/xdb-compose.sh up   (rebuilds the image)"
  echo "     fallback (docker issues): .agents/skills/docker-fallback/xdb-restart.sh build"
  echo "                                .agents/skills/docker-fallback/xdb-restart.sh start"
}

do_typecheck() {
  echo "Running: tsc --noEmit --strict on src/assets/ts/app.ts (esbuild does NOT typecheck)"
  npx --yes -p typescript tsc --noEmit --strict \
      --target es2020 --lib es2020,dom src/assets/ts/app.ts
  echo "OK — typecheck passed."
}

do_harness() {
  local name="${1:-}"
  [ -n "$name" ] || die "harness needs a name, e.g. config-repro.mjs (see 'harnesses')"
  if [ -f "tests/dashboard/$name" ]; then
    echo "Running: node tests/dashboard/$name"
    node "tests/dashboard/$name"
  elif [ -f "$name" ]; then
    echo "Running: node $name (not under tests/dashboard/)"
    node "$name"
  else
    die "no such harness: $name (looked in tests/dashboard/ and cwd)"
  fi
}

do_harnesses() {
  echo "jsdom harnesses in tests/dashboard/:"
  (cd tests/dashboard && ls -1 *.mjs | sed 's/^/  /')
  echo "Run per file from the repo root:  node tests/dashboard/<name>.mjs"
}

usage() { sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; }

cmd="${1:-all}"; [ $# -gt 0 ] && shift
case "$cmd" in
  bundle)    do_bundle ;;
  typecheck) do_typecheck ;;
  all)       do_bundle; do_typecheck ;;
  harness)   do_harness "${1:-}" ;;
  harnesses) do_harnesses ;;
  help|-h|--help) usage ;;
  *) die "unknown subcommand: $cmd — run '$0 help'" ;;
esac