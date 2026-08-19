# Docs index

> **Standing rule:** the docs under `docs/` (`API_REFERENCE.md`,
> `ADMIN_GUIDE.md`, `CONFIGURATION.md`) are user-facing and drift easily.
> After any change that touches routes, permissions/actions, throttling,
> config fields/defaults/clamps, or the dashboard UI, re-check the relevant
> doc and update it in the same pass. `ADMIN_GUIDE.md` and
> `CONFIGURATION.md` in particular had drifted badly before (stale
> throttle sharing, missing `INDEX` action, stale in-memory log ring) —
> keep an eye on them.

| file | contents |
|---|---|
| `README.md` (repo root) | quick start (Docker-first), bare metal `<details>`, route table, "why this shape", Files table |
| `docs/API_REFERENCE.md` | client API only + verified JS/Python examples in `<details>` (incl. index ensure/list/drop step, verified 2026-08-19 in docker); dashboard → points to ADMIN_GUIDE.md#dashboard-api |
| `docs/ADMIN_GUIDE.md` | dashboard views (4-view), ops (login throttles, actions incl. INDEX, log files + filters), dashboard API section, troubleshooting |
| `docs/CONFIGURATION.md` | server.yml + binary config field tables (defaults & clamps), env overrides, adaptive-limit formula, perms format (incl. INDEX action) |
| `authorized_keys.yml.example` | documented permissions template |
| `examples/README.md` | examples crate: full table + options + notes (separate /auth vs dashboard-login throttles, cleanup via delete:true) |
