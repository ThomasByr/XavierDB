#!/usr/bin/env bash
# .agents/skills/docker/xdb-compose.sh
#
# Docker/compose deployment ops (VERIFIED on Docker Desktop 29.7.2, WSL2;
# inotify-over-VirtioFS limitation applies to watcher tests — see SKILL.md).
# Preflights docker + the compose plugin; if broken, point at the bare-metal
# fallback (skills/docker-fallback/SKILL.md).
#
# Usage:
#   xdb-compose.sh up                 docker compose up --build -d
#   xdb-compose.sh watch              docker compose watch (rebuild on src/Cargo.toml)
#   xdb-compose.sh build [--no-cache] rebuild the xavierdb image (cache layers by default)
#   xdb-compose.sh logs [-f]          docker compose logs xavierdb
#   xdb-compose.sh mongo [args]        mongosh shell inside the mongodb service (MongoDB is ALWAYS Docker)
#   xdb-compose.sh password           grep the first-run dashboard password from the logs
#   xdb-compose.sh restart            needed after server.yml changes (read at process start only)
#   xdb-compose.sh ps | down          compose ps / down
#   xdb-compose.sh deploy             repo-root deploy.sh (Linux prod host: pull -> build -> up -f compose.yaml)
#   xdb-compose.sh battery            local rust battery against the docker API (watcher_reload FAILS on Docker Desktop)
#
# PROD vs DEV stacks: plain `docker compose up -d` merges compose.override.yaml
# (dev cargo-watch stack); `deploy.sh` passes -f compose.yaml so prod never
# sees the override.
#
# Overridable env (see .agents/settings/defaults.sh): XDB_REPO, XDB_TEST_MONGO_URI, XDB_MONGO_URI.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${XDB_REPO:=$(cd "$SCRIPT_DIR/../../.." && pwd)}"
# shellcheck disable=SC1091
. "$XDB_REPO/.agents/settings/defaults.sh"
XDB_REPO="$(cd "$XDB_REPO" && pwd)"

cd "$XDB_REPO"
die() { echo "ERROR: $*" >&2; exit 1; }

preflight() {
  command -v docker >/dev/null 2>&1 || die "docker not on PATH — use the bare-metal stack (skills/build-run-test/)"
  docker --version >/dev/null 2>&1 || die "docker not functional"
  docker compose version >/dev/null 2>&1 || die "docker compose plugin missing — without it, use bare metal (skills/build-run-test/)"
}

do_up()    { preflight; docker compose up --build -d; }
do_watch() { preflight; docker compose watch; }
do_build() {
  preflight
  case "${1:-}" in
    --no-cache) docker compose build --no-cache xavierdb ;;
    "")         docker compose build xavierdb ;;
    *)          die "unknown option: $1" ;;
  esac
}
do_logs()   { preflight; docker compose logs "${1:---tail=50}" xavierdb; }
do_passwd() { preflight; docker compose logs xavierdb 2>&1 | rg -i "password" | tail -n 5 || echo "no password line found in the logs — it is printed ONCE at first boot (docker compose logs xavierdb)".; }
do_restart(){ preflight; docker compose restart xavierdb; }
do_ps()     { preflight; docker compose ps; }
do_down()   { preflight; docker compose down; }
do_deploy() { preflight; [ -x "$XDB_REPO/deploy.sh" ] || die "deploy.sh missing at repo root"; echo "Running deploy.sh (prod-style: -f compose.yaml, no override merge)"; bash "$XDB_REPO/deploy.sh"; }
do_mongo()  { preflight; docker compose exec mongodb mongosh "$@"; }
do_battery(){
  preflight
  echo "Running the LOCAL rust battery against the docker API (defaults 127.0.0.1:8000 / localhost mongo already match compose)."
  echo "NOTE: watcher_reload is EXPECTED to FAIL on Docker Desktop (no inotify over VirtioFS)."
  echo "If Docker is broken, use the bare-metal fallback instead (skills/docker-fallback/SKILL.md)."
  (cd "$XDB_REPO" && XDB_TEST_MONGO_URI="${XDB_TEST_MONGO_URI:-$XDB_MONGO_URI}" cargo test)
}

usage() { sed -n '2,23p' "$0" | sed 's/^# \{0,1\}//'; }

cmd="${1:-help}"; [ $# -gt 0 ] && shift
case "$cmd" in
  up)       do_up ;;
  watch)    do_watch ;;
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