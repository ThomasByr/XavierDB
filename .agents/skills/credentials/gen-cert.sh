#!/usr/bin/env bash
# .agents/skills/credentials/gen-cert.sh
#
# Generate a self-signed TLS cert/key pair for server.yml
# tls.cert_path / tls.key_path (self-signed is fine for dev).
#
# Usage:
#   gen-cert.sh -o cert.pem -k key.pem [ -n myhost ] [ -d 365 ]
#
# Options:
#   -o FILE   output cert (PEM)
#   -k FILE   output key  (PEM, unencrypted -nodes)
#   -n NAME   CommonName (default: localhost)
#   -d DAYS   validity (default: 365)
#
# MSYS trap handled: openssl may mangle -subj "/CN=..." via MSYS path
# conversion — we invoke it with MSYS_NO_PATHCONV=1 on Windows. Output paths
# are passed as given (Windows-style paths work: use them directly).
#
# After writing the files, point server.yml at them and restart the server
# (cert + key are hot-reloaded at runtime, but the configured PATHS are read
# at startup — see knowledge/architecture/tls.md).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${XDB_REPO:=$(cd "$SCRIPT_DIR/../../.." && pwd)}"
# shellcheck disable=SC1091
. "$XDB_REPO/.agents/settings/defaults.sh"

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) IS_WINDOWS=1 ;;
  *) IS_WINDOWS=0 ;;
esac

CERT="" KEY="" CN="localhost" DAYS=365
while [ $# -gt 0 ]; do
  case "$1" in
    -o) CERT="${2:-}"; shift 2 ;;
    -k) KEY="${2:-}";  shift 2 ;;
    -n) CN="${2:-}";   shift 2 ;;
    -d) DAYS="${2:-}"; shift 2 ;;
    *) echo "ERROR: unknown option: $1" >&2; exit 2 ;;
  esac
done
[ -n "$CERT" ] && [ -n "$KEY" ] || { echo "ERROR: -o (cert) and -k (key) are required" >&2; exit 2; }
command -v openssl >/dev/null 2>&1 || { echo "ERROR: openssl not on PATH" >&2; exit 1; }

run_openssl() {
  if [ "$IS_WINDOWS" = "1" ]; then MSYS_NO_PATHCONV=1 openssl "$@"; else openssl "$@"; fi
}

run_openssl req -x509 -newkey rsa:2048 -sha256 -nodes \
  -keyout "$KEY" -out "$CERT" \
  -days "$DAYS" -subj "/CN=${CN}/O=XavierDB"
echo "OK — wrote:"
echo "  cert: $CERT"
echo "  key:  $KEY"
echo "Set server.yml tls.cert_path / tls.key_path (or TLS_CERT_PATH / TLS_KEY_PATH) and restart the server."