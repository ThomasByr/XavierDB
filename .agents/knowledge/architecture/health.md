# Architecture — Health

_Split from the former `knowledge/architecture.md` (2026-08-24); section map in `knowledge/architecture/README.md`._

## 7. Health

- `GET /health` (public, cached, default TTL 5s):
  `{status:"ok|degraded|unhealthy", checked_at_ms, next_refresh_seconds,
  compute_latency_ms, qps, max_insert_batch, constants:{max_insert_batch,
  jwt_token_lifetime_seconds, max_document_limit}, app:{status, uptime_s, p50_latency_ms,
  total_requests, active_cursors}, mongodb:{reachable, ping_latency_ms,
  error}}` — 200 only when ok, else 503. `max_insert_batch` is the
  insert-batch cap (server.yml runtime.max_insert_batch), static per process — the battery
  reads it from here so cap-boundary tests work with custom values.
  `constants.jwt_token_lifetime_seconds` mirrors the effective
  config.global.jwt_token_lifetime_minutes × 60 — auth_flow::login_ok
  asserts expires_in against it (the lifetime is dashboard-editable, so the
  test must not hardcode the default).
  `constants.max_document_limit` = config.rate_limit.max_limit — the ceiling
  the enforced per-app limit never exceeds (enforced = clamp(round(internal
  × multiplier × weight), min, max)); limit-bound integration assertions
  must read it instead of assuming 200.
- Verified live: mongod kill → `unhealthy`/`reachable:false`/HTTP 503 (the
  supervised health loop keeps refreshing — no stale "ok"); mongod restart →
  auto-recovery to ok/200 without server restart.

