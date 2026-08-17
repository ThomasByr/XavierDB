#!/bin/bash
set -euo pipefail

git pull origin main
docker compose -f compose.yaml build api
docker compose -f compose.yaml up -d
