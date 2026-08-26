#!/usr/bin/env bash
# .agents/skills/docker/xdb-compose.sh
#
# Docker/compose deployment ops (VERIFIED on Docker Desktop 29.7.2, WSL2;
# inotify-over-VirtioFS limitation applies to watcher tests — see SKILL.md).
# Preflights docker + the compose plugin; if broken, point at the bare-metal
# fallback (skills/docker-fallback/SKILL.md).
#
# The stack is ALWAYS base + one overlay, passed explicitly to compose:
#   XDB_ENV=dev  -> compose.yaml + compose.dev.yaml  (cargo-watch dev loop)
#   XDB_ENV=pre  -> compose.yaml + compose.pre.yaml  (default)
#   XDB_ENV=prod -> compose.yaml + compose.prod.yaml (deploy.sh pins this)
# Because compose is always invoked with -f, no implicit override file is
# ever auto-merged (compose.override.yaml is GONE by design).
#
# Usage:
#   xdb-compose.sh up                 docker compose <base+env> up --build -d
#   xdb-compose.sh build [--no-cache] rebuild the xavierdb image (cache layers by default)
#   xdb-compose.sh logs [-f]          docker compose logs xavierdb
#   xdb-compose.sh mongo [args]        mongosh shell inside the mongodb service (MongoDB is ALWAYS Docker)
#   xdb-compose.sh password           grep the first-run dashboard password from the logs
#   xdb-compose.sh restart            needed after server.yml changes (read at process start only)
#   xdb-compose.sh ps | down          compose ps / down
#   xdb-compose.sh deploy             repo-root deploy.sh (Linux prod host: pull -> build -> up, PROD pair)
#   xdb-compose.sh battery            local rust battery against the docker API (watcher_reload FAILS on Docker Desktop)
#
# Overridable env (see .agents/settings/defaults.sh): XDB_REPO, XDB_ENV,
# XDB_TEST_MONGO_URI, XDB_MONGO_URI.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${XDB_REPO:=$(cd "$SCRIPT_DIR/../../.." && pwd)}"
# shellcheck disable=SC1091
. "$XDB_REPO/.agents/settings/defaults.sh"
XDB_REPO="$(cd "$XDB_REPO" && pwd)"

cd "$XDB_REPO"
die() { echo "ERROR: $*" >&2; exit 1; }

case "${XDB_ENV:-}" in
  "")      XDB_ENV=pre  ;;
  dev)     XDB_ENV=dev  ;;
  pre)     XDB_ENV=pre  ;;
  prod)    XDB_ENV=prod ;;
  *)       die "unknown XDB_ENV '$XDB_ENV' (expected dev | pre | prod)" ;;
esac
# shellcheck disable=SC2086
COMPOSE=(docker compose -f compose.base.yaml -f "compose.${XDB_ENV}.yaml")
echo "compose stack: compose.base.yaml + compose.${XDB_ENV}.yaml (XDB_ENV=${XDB_ENV})"

preflight() {
  command -v docker >/dev/null 2>&1 || die "docker not on PATH — use the bare-metal stack (skills/build-run-test/)"
  docker --version >/dev/null 2>&1 || die "docker not functional"
  docker compose version >/dev/null 2>&1 || die "docker compose plugin missing — without it, use bare metal (skills/build-run-test/)"
}

do_up()     { preflight; "${COMPOSE[@]}" up --build -d; }
do_build() {
  preflight
  case "${1:-}" in
    --no-cache) "${COMPOSE[@]}" build --no-cache xavierdb ;;
    "")         "${COMPOSE[@]}" build xavierdb ;;
    *)          die "unknown option: $1" ;;
  esac
}
do_logs()   { preflight; "${COMPOSE[@]}" logs "${1:---tail=50}" xavierdb; }
do_passwd() { preflight; "${COMPOSE[@]}" logs xavierdb 2>&1 | rg -i "password" | tail -n 5 || echo "no password line found in the logs — it is printed ONCE at first boot (docker compose logs xavierdb)".; }
do_restart(){ preflight; "${COMPOSE[@]}" restart xavierdb; }
do_ps()     { preflight; "${COMPOSE[@]}" ps; }
do_down()   { preflight; "${COMPOSE[@]}" down; }
do_deploy() { preflight; [ -x "$XDB_REPO/deploy.sh" ] || die "deploy.sh missing at repo root"; echo "Running deploy.sh (prod pair: compose.yaml + compose.prod.yaml)"; bash "$XDB_REPO/deploy.sh"; }
do_mongo()  { preflight; "${COMPOSE[@]}" exec mongodb mongosh "$@"; }
do_battery(){
  preflight
  echo "Running the LOCAL rust battery against the docker API (defaults 127.0.0.1:8000 / localhost mongo already match compose)."
  echo "NOTE: watcher_reload is EXPECTED to FAIL on Docker Desktop (no inotify over VirtioFS)."
  echo "If Docker is broken, use the bare-metal fallback instead (skills/docker-fallback/SKILL.md)."
  (cd "$XDB_REPO" && XDB_TEST_MONGO_URI="${XDB_TEST_MONGO_URI:-$XDB_MONGO_URI}" cargo test)
}

usage() { sed -n '2,29p' "$0" | sed 's/^# \{0,1\}//'; }

cmd="${1:-help}"; [ $# -gt 0 ] && shift
case "$cmd" in
  up)       do_up ;;
  build)    do_build "${1:-}" ;;
  logs)     do_logs "${1:-}" ;;
  password) do_passwd ;;
  restart)  do_restart ;;
  ps)       do_ps ;;
  down)     do_down ;;
  mongo)    do_mongo "$@" ;;
  deploy)   do_deploy ;;
  battery)  do_battery ;;
  help|-h|--help) usage ;;
  *) die "unknown subcommand: $cmd — run '$0 help'" ;;
esac
