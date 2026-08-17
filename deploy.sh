#!/bin/bash
git pull origin main
docker compose -f compose.yaml build
docker compose -f compose.yaml up -d
