# .agents/settings/defaults.sh — shared defaults for the .agents/skills/*/ scripts
#
# Source (don't execute) this file from each bash script:
#     # shellcheck disable=SC1091
#     . "$XDB_REPO/.agents/settings/defaults.sh"
# so all tunables live in ONE place and stay overridable.
#
# IMPORTANT — these are TOOL DEFAULTS, not app config. The XavierDB server
# never reads this file; its own settings live in server.yml / the binary
# `config` (see knowledge/config-world.md). Scripts read these values only.
#
# Every variable uses the `${VAR:-default}` form, so a caller can override any
# default by exporting the same XDB_* name in the environment before running a
# script.

# --- repo + server binary -------------------------------------------------
# Repo root. Scripts auto-detect it from their own location; override to force.
XDB_REPO="${XDB_REPO:-}"
# Server binary name WITHOUT the platform suffix (scripts add ".exe" on Windows).
XDB_BIN="${XDB_BIN:-XavierDB}"
# Detached-server log file. Default: $XDB_REPO/target/xdb.log (target/ is
# gitignored and transient). Override with XDB_LOG=/tmp/xdb.log if you prefer.
XDB_LOG="${XDB_LOG:-$XDB_REPO/target/xdb.log}"

# --- docker compose stack --------------------------------------------------
# Which overlay xdb-compose.sh pairs with the base compose.base.yaml: dev | pre
# | prod. Default pre (pre-prod shape: prod image, no watch, named volume).
XDB_ENV="${XDB_ENV:-pre}"

# --- network (mirrors server.yml defaults — see knowledge/config-world.md) --
XDB_HOST="${XDB_HOST:-127.0.0.1}"
XDB_PORT="${XDB_PORT:-8000}"
# Health endpoint used to wait for / verify the server is up.
XDB_HEALTH="${XDB_HEALTH:-http://$XDB_HOST:$XDB_PORT/health}"
# curl --max-time for short health checks (keep short: a long shell can orphan
# a detached fallback server — see skills/docker-fallback/SKILL.md).
XDB_CURL_MAXTIME="${XDB_CURL_MAXTIME:-5}"

# --- MongoDB / integration battery ----------------------------------------
XDB_MONGO_URI="${XDB_MONGO_URI:-mongodb://localhost:27017}"
XDB_TEST_MONGO_URI="${XDB_TEST_MONGO_URI:-$XDB_MONGO_URI}"
# Dashboard credentials for tests/bootstrap.sh (passed from args or these env
# vars; NEVER from a file). Leave empty to be passed per-invocation.
XDB_DASH_USER="${XDB_DASH_USER:-}"
XDB_DASH_PASS="${XDB_DASH_PASS:-}"

# --- perms-watcher snapshot backup dir -------------------------------------
# A byte-identical restore is picked up automatically; snapshots live anywhere.
XDB_SNAPSHOT_DIR="${XDB_SNAPSHOT_DIR:-"$XDB_REPO/target/snapshots"}"
