#!/usr/bin/env bash
# Bootstrap the xavierdb_tb_* fixture world for the integration battery (tests/).
#
# Requires: a RUNNING server + MongoDB (see AGENTS.md §4.1). Idempotent:
# re-running is safe and skips the (slow, ~5 s each) token re-hashing when
# the fixture apps already exist.
#
# Usage:
#   bash tests/bootstrap.sh --dash-user <user> --dash-pass '<password>'
#   XDB_TB_BASE=http://host:8000 bash tests/bootstrap.sh --dash-user ... --dash-pass ...
#
# Credentials are taken ONLY from the --dash-user/--dash-pass arguments (or
# the XDB_DASH_USER/XDB_DASH_PASS env vars) — never from a file.
#
# Caches JWTs + the admin cookie in <temp>/xdb_tb_cache — the exact directory
# tests/common/mod.rs reads (std::env::temp_dir(); git-bash /tmp maps to the
# same place on Windows).
set -u

BASE="${XDB_TB_BASE:-http://127.0.0.1:8000}"
CACHE_DIR="${TMPDIR:-${TMP:-/tmp}}/xdb_tb_cache"
mkdir -p "$CACHE_DIR"
fail() { echo "BOOTSTRAP FAIL: $1" >&2; exit 1; }

# ---- arguments --------------------------------------------------------------
DASH_USER="${XDB_DASH_USER:-}"
DASH_PASS="${XDB_DASH_PASS:-}"
while [ $# -gt 0 ]; do
  case "$1" in
    --dash-user) DASH_USER="${2:-}"; shift 2 ;;
    --dash-pass) DASH_PASS="${2:-}"; shift 2 ;;
    --base-url) BASE="${2:-}"; shift 2 ;;
    *) fail "unknown argument: $1 (use --dash-user and --dash-pass)" ;;
  esac
done
[ -n "$DASH_USER" ] && [ -n "$DASH_PASS" ] \
  || fail "dashboard credentials needed: --dash-user USER --dash-pass PASS (or XDB_DASH_USER/XDB_DASH_PASS)"

# ---- wait for the server ---------------------------------------------------
for _ in $(seq 1 30); do
  if curl -s --max-time 3 "$BASE/health" | grep -q '"status":"ok"'; then break; fi
  sleep 1
done
curl -s --max-time 3 "$BASE/health" | grep -q '"status":"ok"' \
  || fail "server not healthy at $BASE (start MongoDB + the API first)"

# ---- dashboard login -> admin cookie ----------------------------------------
curl -s --max-time 30 -D "$CACHE_DIR/admin.headers" -X POST "$BASE/dashboard/api/login" \
  -H 'Content-Type: application/json' \
  -d "{\"username\":\"$DASH_USER\",\"password\":\"$DASH_PASS\"}" -o "$CACHE_DIR/admin.json" \
  || fail "dashboard login curl"
grep -q '"ok":true' "$CACHE_DIR/admin.json" \
  || fail "dashboard login rejected: $(cat "$CACHE_DIR/admin.json")"
ADMIN_COOKIE="$(grep -i '^set-cookie: xdb_admin' "$CACHE_DIR/admin.headers" | head -1 \
  | sed 's/^[Ss]et-[Cc]ookie: //; s/;.*//')"
[ -n "$ADMIN_COOKIE" ] || fail "no xdb_admin cookie"
echo "$ADMIN_COOKIE" > "$CACHE_DIR/admin.cookie"
echo "dashboard login OK"

# ---- fixture apps (skip when already present) --------------------------------
curl -s --max-time 15 -b "$ADMIN_COOKIE" "$BASE/dashboard/api/perms" -o "$CACHE_DIR/perms.json" \
  || fail "GET perms"
MISSING=""
for app in xdb_tb_main xdb_tb_restricted xdb_tb_ro xdb_tb_m1 xdb_tb_m2 xdb_tb_m3; do
  grep -q "\"app\":\"$app\"" "$CACHE_DIR/perms.json" || MISSING="$MISSING $app"
done
if [ -n "$MISSING" ]; then
  echo "creating apps:$MISSING"
  cat > "$CACHE_DIR/perms_payload.json" <<'JSON'
{"apps":[
 {"app":"xdb_tb_main","set_token":"tb-main-secret-token","allow":[{"actions":["GET","POST","PUT","PATCH","DELETE"],"databases":["*"],"collections":["*"]}],"deny":[]},
 {"app":"xdb_tb_restricted","set_token":"tb-restricted-token","allow":[{"actions":["GET"],"databases":["*"]}],"deny":[{"actions":["GET"],"databases":["xdb_tb_secret"]}]},
 {"app":"xdb_tb_ro","set_token":"tb-ro-secret-token","allow":[{"actions":["GET"],"databases":["xdb_tb_shared"]}],"deny":[]},
 {"app":"xdb_tb_m1","set_token":"tb-m1-secret-token","allow":[{"actions":["GET"],"databases":["xdb_tb_*"]},{"actions":["DELETE"],"databases":["xdb_tb_shared"]}],"deny":[{"actions":["GET"],"databases":["xdb_tb_secret"]}],"names":[{"name":"m1user","deny":[{"actions":["DELETE"],"databases":["xdb_tb_shared"]}]},{"name":"m1user2"}]},
 {"app":"xdb_tb_m2","set_token":"tb-m2-secret-token","allow":[{"actions":["POST","PATCH"],"databases":["xdb_tb_shared"]}],"deny":[]},
 {"app":"xdb_tb_m3","set_token":"tb-m3-secret-token","allow":[{"actions":["GET"],"databases":["xdb_tb_shared"],"collections":["public"]}],"deny":[]}
]}
JSON
  STATUS="$(curl -s --max-time 120 -o "$CACHE_DIR/perms.out" -w '%{http_code}' \
    -b "$ADMIN_COOKIE" -X POST "$BASE/dashboard/api/perms" \
    -H 'Content-Type: application/json' --data @"$CACHE_DIR/perms_payload.json")"
  [ "$STATUS" = "200" ] || fail "perms POST $STATUS: $(cat "$CACHE_DIR/perms.out")"
  echo "apps created"
else
  echo "fixture apps already present"
fi

# ---- logins -> cached JWTs (+ xdb_token cookies) ------------------------------
logins=(
  "tester@xdb_tb_main|tb-main-secret-token|main"
  "tester2@xdb_tb_main|tb-main-secret-token|main2"
  "ruser@xdb_tb_restricted|tb-restricted-token|ruser"
  "reader@xdb_tb_ro|tb-ro-secret-token|reader"
  "reader2@xdb_tb_ro|tb-ro-secret-token|reader2"
  "m1user@xdb_tb_m1|tb-m1-secret-token|m1user"
  "m1user2@xdb_tb_m1|tb-m1-secret-token|m1user2"
  "u2@xdb_tb_m2|tb-m2-secret-token|u2"
  "u3@xdb_tb_m3|tb-m3-secret-token|u3"
)
for entry in "${logins[@]}"; do
  id="${entry%%|*}"; rest="${entry#*|}"; tok="${rest%%|*}"; key="${rest#*|}"
  [ -s "$CACHE_DIR/$key.jwt" ] && continue   # cached JWT already present
  curl -s --max-time 30 -D "$CACHE_DIR/$key.headers" -X POST "$BASE/auth" \
    -H 'Content-Type: application/json' \
    -d "{\"identifier\":\"$id\",\"token\":\"$tok\"}" -o "$CACHE_DIR/$key.json" \
    || fail "auth curl $id"
  grep -q '"token":"' "$CACHE_DIR/$key.json" \
    || fail "auth rejected $id: $(cat "$CACHE_DIR/$key.json")"
  grep -o '"token":"[^"]*"' "$CACHE_DIR/$key.json" | head -1 \
    | sed 's/"token":"//;s/"//' > "$CACHE_DIR/$key.jwt"
  grep -i '^set-cookie: xdb_token' "$CACHE_DIR/$key.headers" | head -1 \
    | sed 's/^[Ss]et-[Cc]ookie: //; s/;.*//' > "$CACHE_DIR/$key.cookie" || true
  echo "login OK: $id"
done

# ---- seed the fixture dbs so /ls and restricted-app tests see them -----------
MAIN_JWT="$(cat "$CACHE_DIR/main.jwt")"
for db in xdb_tb_shared xdb_tb_secret xdb_tb_extra xdb_tb_crud xdb_tb_query \
          xdb_tb_proj xdb_tb_page xdb_tb_edge; do
  code="$(curl -s --max-time 15 -o /dev/null -w '%{http_code}' \
    -H "Authorization: Bearer $MAIN_JWT" -X POST "$BASE/q/$db/seed" \
    -H 'Content-Type: application/json' -d '{"data":{"_id":"seed-1","v":1}}')"
  [ "$code" = "201" ] || [ "$code" = "409" ] || fail "seed $db -> $code"
done
echo "dbs seeded"
echo "BOOTSTRAP OK — cache in $CACHE_DIR (JWT TTL 90 min; refresh is automatic)"
