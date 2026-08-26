#!/bin/bash
# Prod deployment (Linux VPS): pull, build, up — pinned to the PROD compose
# pair (base + prod overlay). Resource tuning lives in .env (see
# .env.example); the mongo data bind-mount path is XAVIER_MONGO_DATA
# (default ${HOME}/data/xavier-mongo-db). One-time migration from the old
# named-volume stack: see the comment block in compose.prod.yaml.
set -euo pipefail

git pull origin main
docker compose -f compose.base.yaml -f compose.prod.yaml build xavierdb
docker compose -f compose.base.yaml -f compose.prod.yaml up -d
