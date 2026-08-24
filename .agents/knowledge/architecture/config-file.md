# Architecture — Binary config file

_Split from the former `knowledge/architecture.md` (2026-08-24); section map in `knowledge/architecture/README.md`._

## 6. Config file (config.rs)

- `XDB1` magic + crc32 + len + bincode; unknown version refused → backup fallback.
  Atomic writes (tmp + fsync + rename); backups `config.bak`, `config.bak.2`,
  … rotate (MAX_BACKUPS=5, chain is real: oldest dropped, rest shifted, fresh
  copy — verified by test). History capped at 10k snapshots `{ts, desc, path,
  snapshot, by}`; **snapshots are FLAT (no history/redo inside)** — nesting
  them would double the file size on every mutation; undo/redo/revert rebuild
  the entry list from metadata. API returns history NEWEST-first and
  `revert {index}` takes the NEWEST-FIRST display position (0 = newest).
- Sanitization: `ConfigFile::sanitize()` (config.rs) is the single source,
  applied on save/import/revert AND `load_from_disk` (a corrupted config with
  an OOM-able max_limit or an overflowing jwt lifetime/session ttl can
  otherwise be loaded). Exact clamps: min_limit ≥ 1 and ≤ 10 000;
  max_limit ∈ [min_limit, 10 000] (min > max would panic the metrics loop —
  max is raised to min); multiplier ∈ [0.05, 20]; target_latency ∈ [1, 60 000];
  growth ∈ [1, 2]; tick ∈ [1, 3600]; ema ∈ [0.01, 0.9]; sensitivities ∈
  [0, 20]; health ttl ∈ [1, 3600]; log_level ∈ {info, debug};
  per-ip ∈ [1, 10 000]; session ttl ∈ [1, 720]; jwt ∈ [1, 43 200].
- Key fields (defaults): global{jwt_token_lifetime_minutes=90,
  permission_file="authorized_keys.yml"}, rate_limit{min=1, max=200,
  multiplier=1.0, target=50, pressure_sens=1.5, latency_sens=1.0, growth=1.15,
  tick=5, ema=0.2, weights{}}, health{ttl=5}, dashboard{log_level="info"},
  auth{per_ip=30, session_ttl_h=24}, blocked[], history[],
  redo[].
- 2026-08-21: `dashboard.poll_seconds`, `dashboard.graph_smoothing` and
  `dashboard.theme` were REMOVED from ConfigFile (theme/smoothing/poll
  interval are per-browser dashboard prefs in localStorage — see below).
  Bincode is positional → pre-removal `config` files fail to decode and fall
  back to defaults (accepted, no migration; old files were rm'd by hand).
- `dashboard.poll_seconds` was u64 → f64 (2026-08-14); bincode used VARINT int
  encoding → legacy config files failed to decode → defaults (historical note;
  field since removed).

