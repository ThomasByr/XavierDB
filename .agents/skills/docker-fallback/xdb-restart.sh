#!/usr/bin/env bash
# .agents/skills/docker-fallback/xdb-restart.sh
#
# Bare-metal server restart ritual (kill / build / start / test) as SEPARATE
# commands.
#
# ACTIVATE ONLY when the Docker default fails (see SKILL.md): this is the
# fallback path for the XavierDB server binary — MongoDB stays in Docker.
#
# Some OSes refuse to overwrite a running executable ("Accès refusé" here),
# so `cargo build` fails until the server is killed. Keep each step a SEPARATE
# invocation: a shell that times out can kill a disowned server, so `start` is
# its own command (see SKILL.md for the full trap list — never run plain
# `cargo build` between `cargo build --tests` and `cargo test`).
#
# Usage:
#   xdb-restart.sh kill                     stop the server by process name
#   xdb-restart.sh kill-port <port>         stop ONLY the instance on <port> (e.g. 8443)
#   xdb-restart.sh build                    cargo build --tests (needs server down on some OSes)
#   xdb-restart.sh start                    start detached (own command), wait for /health
#   xdb-restart.sh test [<area>]            cargo test (or --test <area>); sets XDB_TEST_MONGO_URI
#   xdb-restart.sh cycle                    kill -> build --tests -> start -> test (convenience)
#   xdb-restart.sh status                   is /health answering?
#   xdb-restart.sh help
#
# Overridable env (see .agents/settings/defaults.sh): XDB_REPO, XDB_BIN,
# XDB_HOST, XDB_PORT, XDB_HEALTH, XDB_LOG, XDB_CURL_MAXTIME, XDB_MONGO_URI,
# XDB_TEST_MONGO_URI, XDB_START_WAIT.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${XDB_REPO:=$(cd "$SCRIPT_DIR/../../.." && pwd)}"
# shellcheck disable=SC1091
. "$XDB_REPO/.agents/settings/defaults.sh"
XDB_REPO="$(cd "$XDB_REPO" && pwd)"

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) IS_WINDOWS=1 ;;
  *) IS_WINDOWS=0 ;;
esac

BIN="$XDB_REPO/target/debug/$XDB_BIN"
[ "$IS_WINDOWS" = "1" ] && BIN="$BIN.exe"
: "${XDB_START_WAIT:=45}"

# --- helpers ---------------------------------------------------------------
health_up() { curl -fsS --max-time "$XDB_CURL_MAXTIME" "$XDB_HEALTH" >/dev/null 2>&1; }

wait_server_up() {
  local n="${1:-30}" i
  for i in $(seq 1 "$n"); do health_up && return 0; sleep 0.5; done
  return 1
}

wait_server_down() {
  local n="${1:-20}" i
  for i in $(seq 1 "$n"); do health_up || return 0; sleep 0.5; done
  return 1
}

die() { echo "ERROR: $*" >&2; exit 1; }

# --- subcommands ------------------------------------------------------------
do_kill() {
  echo "Stopping $XDB_BIN (by process name)..."
  if [ "$IS_WINDOWS" = "1" ]; then
    taskkill //F //IM "$XDB_BIN.exe" >/dev/null 2>&1 || true
  else
    pkill -x "$XDB_BIN" >/dev/null 2>&1 || pkill -f "target/debug/$XDB_BIN" >/dev/null 2>&1 || true
  fi
  if wait_server_down; then
    echo "Down — $XDB_HEALTH no longer answers."
  else
    echo "WARN: $XDB_HEALTH still answers — check whether a second instance is running (use 'kill-port')." >&2
  fi
}

do_kill_port() {
  local port="$1" pids
  [ -n "$port" ] || die "kill-port needs a port (e.g. 8443)"
  echo "Stopping the instance on port $port (by PID — never by process name when a second instance exists)..."
  if [ "$IS_WINDOWS" = "1" ]; then
    pids="$(netstat -ano | awk -v p=":$port" '$2 ~ p "$" && $4 ~ /LISTEN/ {print $5}' | sort -u || true)"
    if [ -z "$pids" ]; then echo "Nothing listening on :$port."; return 0; fi
    for pid in $pids; do taskkill //F //PID "$pid" >/dev/null 2>&1 || true; done
  else
    if command -v lsof >/dev/null 2>&1; then
      pids="$(lsof -ti :"$port" 2>/dev/null || true)"
      [ -z "$pids" ] && { echo "Nothing listening on :$port."; return 0; }
      # shellcheck disable=SC2086
      kill $pids 2>/dev/null || true
    else
      die "lsof not found — install it or kill by PID manually"
    fi
  fi
  echo "Done. Verify with: $0 status  /  $( [ "$IS_WINDOWS" = 1 ] && echo "netstat -ano | rg :$port" || echo "lsof -i :$port" )"
}

do_build() {
  if health_up; then
    echo "WARN: server appears UP at $XDB_HEALTH — some OSes refuse to overwrite a running exe; run '$0 kill' first." >&2
  fi
  echo "Running: cargo build --tests (rebuilds the server binary AND keeps test-mode fingerprints fresh)..."
  (cd "$XDB_REPO" && cargo build --tests)
  echo "OK — $BIN rebuilt."
}

# `start` must stay its own command (trap: a long shell can kill the disowned
# server). We detach with nohup (+ setsid on POSIX) so it survives this exit.
do_start() {
  [ -x "$BIN" ] || die "$BIN not found — run '$0 build' first"
  mkdir -p "$(dirname "$XDB_LOG")"
  echo "Starting $BIN detached — log: $XDB_LOG"
  if [ "$IS_WINDOWS" = "1" ]; then
    ( nohup "$BIN" >>"$XDB_LOG" 2>&1 & disown ) || true
  elif command -v setsid >/dev/null 2>&1; then
    setsid "$BIN" >>"$XDB_LOG" 2>&1 < /dev/null & disown || true
  else
    nohup "$BIN" >>"$XDB_LOG" 2>&1 & disown || true
  fi
  if wait_server_up "$XDB_START_WAIT"; then
    echo "OK — server answering at $XDB_HEALTH (pid landed in $XDB_LOG on first boot)."
  else
    echo "WARN: no answer at $XDB_HEALTH after ${XDB_START_WAIT}s — last log lines:" >&2
    tail -n 20 "$XDB_LOG" >&2 || true
    exit 1
  fi
}

do_test() {
  local area="${1:-}"
  echo "Running: cargo test ${area:+--test $area} (XDB_TEST_MONGO_URI=${XDB_TEST_MONGO_URI:-$XDB_MONGO_URI} — enables the env-gated Mongo-backed unit tests)"
  (cd "$XDB_REPO" && XDB_TEST_MONGO_URI="${XDB_TEST_MONGO_URI:-$XDB_MONGO_URI}" cargo test ${area:+--test "$area"})
}

do_cycle() {
  do_kill
  do_build
  do_start
  do_test "${1:-}"
}

do_status() {
  if health_up; then
    echo "UP   — $XDB_HEALTH answers"
  else
    echo "DOWN — $XDB_HEALTH does not answer"
  fi
  echo "binary: $BIN ($( [ -x "$BIN" ] && echo present || echo MISSING ))  |  log: $XDB_LOG"
}

usage() { sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; }

# --- main -------------------------------------------------------------------
cmd="${1:-help}"; shift || true
case "$cmd" in
  kill)      do_kill ;;
  kill-port) do_kill_port "${1:-}" ;;
  build)     do_build ;;
  start)     do_start ;;
  test)      do_test "${1:-}" ;;
  cycle)     do_cycle "${1:-}" ;;
  status)    do_status ;;
  help|-h|--help) usage ;;
  *) die "unknown subcommand: $cmd — run '$0 help'" ;;
esac