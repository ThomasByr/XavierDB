#!/usr/bin/env bash
# .agents/skills/build-run-test/battery.sh
#
# Integration battery: one-time fixture bootstrap + cargo test.
# BLACK-BOX HTTP tests — they target the DOCKER stack by default (API + MongoDB
# in containers: `xdb-compose.sh up`), or the bare-metal fallback server when
# Docker has issues (skills/docker-fallback/SKILL.md). MongoDB is ALWAYS Docker.
#
# Usage:
#   battery.sh bootstrap [--dash-user U] [--dash-pass P]   one-time fixture setup (idempotent)
#   battery.sh run                                         full cargo test (default)
#   battery.sh single <area>                               one test area (--test <area>)
#   battery.sh all                                         bootstrap + run
#
# Credentials for bootstrap come ONLY from --dash-user/--dash-pass args or the
# XDB_DASH_USER/XDB_DASH_PASS env vars — never from a file. They are the
# dashboard (server.yml admin) credentials; read them from
# .pi/notes/credentials.md when unsure.
#
# Overridable env (see .agents/settings/defaults.sh): XDB_REPO, XDB_BIN,
# XDB_HOST, XDB_PORT, XDB_HEALTH, XDB_CURL_MAXTIME, XDB_MONGO_URI,
# XDB_TEST_MONGO_URI, XDB_DASH_USER, XDB_DASH_PASS.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${XDB_REPO:=$(cd "$SCRIPT_DIR/../../.." && pwd)}"
# shellcheck disable=SC1091
. "$XDB_REPO/.agents/settings/defaults.sh"
XDB_REPO="$(cd "$XDB_REPO" && pwd)"

cd "$XDB_REPO"
health_up() { curl -fsS --max-time "$XDB_CURL_MAXTIME" "$XDB_HEALTH" >/dev/null 2>&1; }
die() { echo "ERROR: $*" >&2; exit 1; }

preflight() {
  health_up || die "server not answering at $XDB_HEALTH — start it first: 'xdb-compose.sh up' (default), or the bare-metal fallback 'skills/docker-fallback/xdb-restart.sh start' when Docker has issues"
  # NOTE: mongod is NOT preflighted (no HTTP probe); a missing Mongo fails the
  # tests loudly, and tests/common/mod.rs documents XDB_TB_MONGO_URI.
  echo "Preflight OK — server up at $XDB_HEALTH."
}

do_bootstrap() {
  local user="${XDB_DASH_USER:-}" pass="${XDB_DASH_PASS:-}"
  while [ $# -gt 0 ]; do
    case "$1" in
      --dash-user) user="${2:-}"; shift 2 ;;
      --dash-pass) pass="${2:-}"; shift 2 ;;
      *) die "unknown option: $1" ;;
    esac
  done
  [ -n "$user" ] || die "--dash-user missing (or export XDB_DASH_USER) — dashboard creds live in .pi/notes/credentials.md"
  [ -n "$pass" ] || die "--dash-pass missing (or export XDB_DASH_PASS)"
  preflight
  echo "Running: bash tests/bootstrap.sh --dash-user <user> --dash-pass <pass>"
  bash tests/bootstrap.sh --dash-user "$user" --dash-pass "$pass"
  echo "OK — fixtures in place (idempotent; slow steps skipped when done)."
}

do_run() {
  preflight
  local area="${1:-}"
  echo "Running: cargo test ${area:+--test $area} (XDB_TEST_MONGO_URI=${XDB_TEST_MONGO_URI:-$XDB_MONGO_URI})"
  (cd "$XDB_REPO" && XDB_TEST_MONGO_URI="${XDB_TEST_MONGO_URI:-$XDB_MONGO_URI}" cargo test ${area:+--test "$area"})
}

usage() { sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; }

cmd="${1:-run}"; [ $# -gt 0 ] && shift
case "$cmd" in
  bootstrap) do_bootstrap "$@" ;;
  run)       do_run "${1:-}" ;;
  single)    [ -n "${1:-}" ] || die "single needs an area (e.g. multi_app) — area names: see SKILL.md"; do_run "$1" ;;
  all)       do_bootstrap "$@"; do_run ;;
  help|-h|--help) usage ;;
  *) die "unknown subcommand: $cmd — run '$0 help'" ;;
esac