#!/usr/bin/env bash
# .agents/skills/examples/examples.sh
#
# Examples crate helper (examples/ is its OWN crate + lockfile — independent
# of the server build; deps are ureq + serde_json only, no clap).
#
# Each example = setup_<name>.rs (dashboard API: admin login + perms POST) +
# <name>.rs (client API showcase). Setup bins REQUIRE --admin-user/--admin-pass
# (dashboard creds = server.yml admin.username, default "admin"; re-running a
# setup is idempotent — it refreshes the token hash).
#
# Usage:
#   examples.sh build                         cargo build (manifest-path examples/Cargo.toml)
#   examples.sh list                          show the 9 bin pairs
#   examples.sh run <bin> [-- args...]        cargo run a bin, args passed through
#   examples.sh run setup_projection -- --admin-user admin --admin-pass <secret>
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

do_build() {
  echo "Running: cargo build --manifest-path examples/Cargo.toml"
  cargo build --manifest-path examples/Cargo.toml
  echo "OK — examples built (own lockfile; never touches the server build)."
}

do_list() {
  echo "Examples (setup_<name>.rs = dashboard perms setup, <name>.rs = client showcase):"
  (cd examples/src/bin && ls -1 *.rs | sed 's/\.rs$//' | sort -u | sed 's/^/  /')
}

do_run() {
  local bin_name="${1:-}"
  [ -n "$bin_name" ] || die "run needs a bin name (see 'list')"
  [ -f "examples/src/bin/$bin_name.rs" ] || die "no examples/src/bin/$bin_name.rs — see 'list'"
  shift
  echo "Running: cargo run --manifest-path examples/Cargo.toml --bin $bin_name -- $*"
  cargo run --manifest-path examples/Cargo.toml --bin "$bin_name" -- "$@"
}

usage() { sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; }

cmd="${1:-help}"; [ $# -gt 0 ] && shift
case "$cmd" in
  build) do_build ;;
  list)  do_list ;;
  run)   do_run "$@" ;;
  help|-h|--help) usage ;;
  *) die "unknown subcommand: $cmd — run '$0 help'" ;;
esac