#!/usr/bin/env bash
# .agents/skills/build-run-test/build.sh
#
# Host-side build pipeline (dashboard bundle -> typecheck -> server binary).
# NOTE: the DEFAULT build is the Docker image (`skills/docker/xdb-compose.sh
# up --build`). This script serves the FALLBACK (docker issues, see
# skills/docker-fallback/SKILL.md) and low-level host Rust checks; the `bundle`
# + `typecheck` steps are also useful host-side before an image build (esbuild
# runs on the host either way).
#
# Usage:
#   build.sh bundle       npm install (if needed) + npm run build (esbuild TS -> app.js)
#   build.sh typecheck    tsc --noEmit on the dashboard TS (esbuild does NOT typecheck)
#   build.sh server       cargo build [--release]  (fallback: fails while the server runs on some OSes — docker-fallback skill)
#   build.sh all          bundle + typecheck + server   (default)
#
# Options:
#   --release   build the release binary (server subcommand / all)
#
# Overridable env (see .agents/settings/defaults.sh): XDB_REPO, XDB_BIN,
# XDB_HOST, XDB_PORT, XDB_HEALTH, XDB_CURL_MAXTIME.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${XDB_REPO:=$(cd "$SCRIPT_DIR/../../.." && pwd)}"
# shellcheck disable=SC1091
. "$XDB_REPO/.agents/settings/defaults.sh"
XDB_REPO="$(cd "$XDB_REPO" && pwd)"

cd "$XDB_REPO"
PROFILE="debug"
ARGS=()
for a in "$@"; do
  case "$a" in
    --release) PROFILE="release" ;;
    *) ARGS+=("$a") ;;
  esac
done

health_up() { curl -fsS --max-time "$XDB_CURL_MAXTIME" "$XDB_HEALTH" >/dev/null 2>&1; }
die() { echo "ERROR: $*" >&2; exit 1; }

do_bundle() {
  if [ ! -d node_modules ]; then
    echo "node_modules missing — running npm install"
    npm install
  fi
  echo "Running: npm run build (esbuild ts/app.ts -> src/assets/app.js)"
  npm run build
  echo "OK — bundle rebuilt. A SERVER REBUILD follows, because app.js is"
  echo "     include_str!-embedded at compile time (see xdb-restart.sh)."
}

do_typecheck() {
  echo "Running: tsc --noEmit --strict on src/assets/ts/app.ts (esbuild does NOT typecheck)"
  npx --yes -p typescript tsc --noEmit --strict \
      --target es2020 --lib es2020,dom src/assets/ts/app.ts
  echo "OK — typecheck passed."
}

do_server() {
  if health_up; then
    echo "WARN: server appears UP at $XDB_HEALTH — some OSes refuse to overwrite a running exe; kill it first (xdb-restart.sh kill)." >&2
  fi
  if [ "$PROFILE" = "release" ]; then
    echo "Running: cargo build --release"
    cargo build --release
    echo "OK — target/release/$XDB_BIN built."
  else
    echo "Running: cargo build"
    cargo build
    echo "OK — target/debug/$XDB_BIN built."
  fi
}

do_all() {
  echo "==> bundle (dashboard assets)"
  do_bundle
  echo "==> typecheck (dashboard TS)"
  do_typecheck
  echo "==> server build ($PROFILE)"
  do_server
}

usage() { sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'; }

cmd="${1:-all}"; [ $# -gt 0 ] && shift
case "$cmd" in
  bundle)    do_bundle ;;
  typecheck) do_typecheck ;;
  server)    do_server ;;
  all)       do_all ;;
  help|-h|--help) usage ;;
  *) die "unknown subcommand: $cmd — run '$0 help'" ;;
esac