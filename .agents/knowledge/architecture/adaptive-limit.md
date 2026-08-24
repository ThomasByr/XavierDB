# Architecture — Adaptive limit & system sampling

_Split from the former `knowledge/architecture.md` (2026-08-24); section map in `knowledge/architecture/README.md`._

## 5. Adaptive limit (metrics.rs)

### System sampling (container-aware since 2026-08-19)

`metrics_loop` samples CPU/RAM/disk/network every tick into `SystemSnapshot`
(for `/dashboard/api/metrics` and the pressure input below). Inside a
container, /proc exposes the HOST's view, so all three are cgroup-aware on
Linux, falling back to sysinfo host-wide values when no limit is set (bare
metal, Windows):

- **RAM**: `sys.cgroup_limits()` (sysinfo reads cgroup v2 `memory.max`/
  `memory.current`, v1 equivalent). Used = limit − free (includes reclaimable
  page cache — same trade-off as `docker stats`). Fallback when limit ≥ host
  total or no cgroup (Windows: API returns None).
- **CPU**: hand-rolled cgroup reader (`cgroup_cpu_pct` in metrics.rs —
  sysinfo has NO cgroup CPU support). Δ`usage_usec` / (elapsed × effective
  cores from `cpu.max` "quota period" / v1 `cpu.cfs_quota_us`/`period`).
  Quota "max"/-1 → fallback to `sys.global_cpu_usage()` (host-wide /proc/stat).
  First tick only seeds the baseline (falls back once).
- **Disk**: ONE filesystem, not a sum over all mounts (summing double-counts
  the host fs once per bind mount inside a container — the old "107/368 GB on
  a 74G disk" bug). Longest component-prefix mount match (`best_mount`/
  `pick_disk`) on the target path: env `DISK_PATH` override, default = app
  cwd (repo root = `/app` in Docker, same host filesystem as the bind-mounted
  Mongo data dir on prod). Zero matches → metrics stay 0 + one WARN log.
- **Network**: unchanged — /proc/net/dev is already container-scoped under
  the default bridge network (own netns); counts container↔mongo traffic
  too. Would become host-wide under `network_mode: host`.

These feed `pressure`, so the adaptive limit reacts to the container's OWN
limits (e.g. `memory: 0.5g`, `cpus: "1.0"` in compose.yaml), not the VPS's.

### Adaptive formula

- Per-app document limit, re-derived every tick (default 5s):
  `lat_err = max(0,(p50−target)/target)`, `pressure = max(0,(cpu−60)/40,
  (mem−70)/30)`, `shrink = 1/(1+K_l·lat_err+K_p·pressure)`; internal limit ×=
  shrink if <1 else × growth_rate, clamped [min_limit, max_limit]; enforced =
  `round(internal · multiplier · weight).clamp(min,max)`. Internal STARTS at
  max_limit on first tick. Per-app `weight` in `config.rate_limit.weights`
  (0.1–10, dashboard-editable, default 1.0). Higher weight = bigger share of
  the page limit under load (never above max_limit).
- Rates are delta-based: `ClientStats.last_total` cumulative counters, EMA
  smoothing (alpha = `config.rate_limit.ema_alpha`), decay to 0 when idle;
  history = 120 samples per tick. Both app AND name keys get rates/sparklines;
  adaptive limit is app-only (`key[4..]` strips the `app:` prefix).
- Requests over the limit: first page + `next_cursor` (client must paginate;
  the server never loads a huge set into RAM).

