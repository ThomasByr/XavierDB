#!/bin/bash
set -euo pipefail

git pull origin main
docker compose -f compose.yaml build xavierdb
docker compose -f compose.yaml up -d
