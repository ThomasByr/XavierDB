# Architecture — Dashboard & request log formats

_Split from the former `knowledge/architecture.md` (2026-08-24); section map in `knowledge/architecture/README.md`._

## 9. Dashboard

- Embedded SPA (`include_str!` at compile time, served no-cache under
  `/dashboard/`), hash-routed, 4 pages: `#/overview | #/clients | #/config |
  #/logs`. Permissions/rate-limit pages were removed (2026-08 rework).
- TS source `src/assets/ts/*.ts` (11 modules, zero deps — see
  .agents/skills/dashboard-rebuild/SKILL.md for the file map) → esbuild → `src/assets/app.js`
  (generated, never hand-edit). No JS libs, no external fonts.
- Full dashboard API surface (all under `/dashboard/api/*`, `xdb_admin`
  session cookie; errors same `{error, code, status}` shape): login/logout/
  session, metrics (big poll payload), block/unblock, app_weight, perms
  GET/POST(full-merge)/reload, databases, config GET/POST/undo/redo/reload/
  reset/revert/export/import, logs (rotating FILES on disk, server.yml-configured
  log.files/log.size_mb — no in-memory ring; ?limit&before paging + app/name
  facets; every console line incl. eprintln/panics). Contracts: see api.md.
- Config tab: EXPLICIT save — slider edits alone don't persist (a page
  reload discards them); an amber "unsaved changes" dirty pill is pinned to
  the card title line (`margin-left:auto` inside the flex `h3` — never in the
  buttons row, where a full wrapping line leaves no free space and the pill
  lands inline between Save/Undo), Save is disabled while clean, and an
  in-flight `configSaving` guard prevents double POSTs.
- Logs box colors are theme-aware `--logs-*` tokens, defined in ALL THREE
  theme blocks (`:root` light, `prefers-color-scheme: dark`, forced
  `[data-theme="dark"]`) — any new theme-aware token must land in all three.
- Browser-behavior debugging without a browser: a jsdom repro drives the
  SERVED bundle (fetch `/dashboard/` index.html + app.js — re-fetch after
  EVERY rebuild, the embed is compile-time), stubs fetch/matchMedia, and
  simulates clicks. Pattern: see skills/dashboard-rebuild/SKILL.md.

### Dashboard UI architecture (src/assets/ts/ — one module per tab)

- Topbar: `.mongo-widget` = pill containing `#mongo-btn` (`.mongo-status`:
  `#mongo-dot` + "MongoDB status" text) and `#mongo-refresh` (↻ INSIDE the
  pill). Dot maps /health status ok → `.ok` green, degraded → `.warn`
  orange, unhealthy → `.bad` red via `updateMongoStatus(h)` (called from
  BOTH `renderOverviewData` and `renderClientsData`); tooltip carries ping
  latency / error. `refreshMongoStatus()` fetches `/health` directly (public
  root route, NOT via `api()` which prefixes `/dashboard/api`), updates
  `lastMetrics.health` + the dot, returns the doc. `#mongo-refresh` = silent
  refresh; `#mongo-btn` click = same + snackbar with fresh status/ping. The
  old standalone `#refresh-btn` (metrics poll) is gone.

- Overview: blocked-apps alert strip (`.ov-alert`, hidden while no app is
  blocked; `renderOvAlert` lists the blocked apps as `.badge.bad`), 4 stat
  chips + 5 mini chart cards (CPU, Memory, Disk, Download, Upload) via
  `drawMini()`, an "All apps · RPS" card (`#ov-rps`, one line per app_id on a
  SHARED y-scale, stable per-app colors via `lineColor` string hash, legend
  with current rps; window button under the chart opens the `.win-pop`
  slider popover — 16 presets 1 min → 1 year, persisted in localStorage
  `xdb-rps-window`), plus an "App traffic" card (`renderOvTraffic`): the top 6
  apps by RPS (`OV_TOP_APPS`), rebuilt every poll like the limits table —
  columns weight / trend (70×22 sparkline from `rps_history`, drawn only
  AFTER the row is attached — `clientWidth` is 0 before) / rps / p50 / limit
  / status badge; header summary = active count, summed rps, worst p50,
  lifetime `health.app.total_requests`.
- RPS chart "Show details" breakdown (2026-08-18): a `#ov-rps-details`
  button (top right of the chart card, in `.rps-head` next to the legend)
  opens the `.det-pop` popover — one switch per app_id (multi-select,
  persisted in localStorage `xdb-rps-details`; button label shows the
  selection count). For each selected app the chart draws a STACKED
  name_id breakdown under the app line: name_ids ranked by average
  contribution over the displayed window (biggest contributor at the
  BOTTOM), band i = filled area between cumulative level i-1 and i
  (sum of names 0..i at each sample time, so the top level ≈ the app
  line, which is drawn separately at full opacity and is never
  stroke-duplicated as a stack level). Line alpha 0.85 → 0.30 bottom-up,
  band fill 0.50 → 0.10 bottom-up (2026-08-20, was 0.30 → 0.08).
  Contribution threshold (2026-08-20): name_ids under a % share of the
  app's window-average traffic merge into ONE synthetic hatched band
  `others (N)` (diagonal-hatch CanvasPattern per app color, fallback
  translucent fill); the top contributor is always kept. The threshold
  slider lives INSIDE the "Show details" popover (below the app switch
  list, `.dp-thr-row` — 0–100 step 1, live re-render while the popover
  stays open, persisted in localStorage `xdb-rps-threshold`, default 33 →
  at most ~3 individual bands; 0 = every band; there is NO standalone
  button in `.rps-head`). The hover
  tooltip mirrors the chart: it lists only the DRAWN bands
  (`bandPts` on NameStack — kept name_ids + the aggregated `others (N)`
  row; changed 2026-08-20, was every name_id via `allNames`/`allPts` —
  giant unmoving-with-cursor panels). Set the threshold to 0 to get
  every band in both chart and tooltip. name_id labels are drawn INSIDE
  the right edge of the plot, each just below its own cumulative line (min 12 px apart,
  ellipsized to 170px) — NOT in the top legend. Chart hover: dashed
  vertical crosshair + light tooltip panel (Chart.js-style) listing every
  app row with its interpolated rps, name_id rows nested under expanded
  apps (indented, slightly offset, with a vertical app-color bar spanning
  the group). Hover redraws from cached draw args (`rpsDrawArgs`), no
  re-poll needed. jsdom verification: `threshold-repro.mjs` (see
  skills/dashboard-rebuild/SKILL.md).
- RPS chart Focus mode (2026-08-21): the RPS card header carries a
  Global ⇄ Focus segmented switch (`.rps-mode`, centered between the
  `#ov-rps-title` span and the summary — two `margin-left:auto` children
  split the h3 free space), with a sliding thumb (`.rm-thumb`, CSS
  transform on `[data-mode]`) and a `▾` arrow (`.rm-arrow`, always
  clickable) that opens the app picker `.focus-pop` (det-pop look,
  single-select rows: swatch + name + ✓ on the selected one, anchored
  `position:absolute` centered under the `.rps-mode` wrapper which is the
  positioning context). Global = one line per app (+ optional stacked
  breakdown). Focus = one line per name_id of ONE app: same archive
  (`name:<id>@<app>` keys), same window selector, same shared scale,
  same hover tooltip (rows = name series), same legend mechanism (swatch
  + current rps per name, sorted); NO stacked bands (stacks = []). In
  Focus the card title becomes `<app> · RPS`, the summary counts
  `name_id(s)` ("no name_id series yet" / "no app selected — pick one
  with the ▾ arrow" when empty), and the `#ov-rps-details` button is
  DISABLED (the breakdown is a Global feature; guard in its onclick too —
  Focus already plots every name_id). Persisted per-client in
  localStorage: `xdb-rps-mode` ("global"|"focus") and `xdb-rps-focus`
  (the app id). Switching to Focus with no saved app auto-opens the
  picker; the picker offers live apps plus a saved-but-not-live
  selection. jsdom verification: `focus-repro.mjs` (see
  skills/dashboard-rebuild/SKILL.md).
- RPS long-window archive (`RpsArchive`, ts/rps-archive.ts): the server only
  serves ~120 ticks of `rps_history`, so the dashboard samples every /metrics
  poll into tiered average buckets (1s/4s/12s/1m/5m/15m/30m/2h/6h/1d
  resolutions; since 2026-08-24 the tier ladder is BACKEND-TICK-INDEPENDENT
  — the finest tier buckets at 1 s, the practical cap being the dashboard
  poll interval, default 2 s) and persists them to localStorage
  `xdb-rps-archive-v2` (saved ≥ every 30 s + on unload; apps unseen for 40 d
  pruned; a v1 archive auto-migrates at load: points flattened and
  re-bucketed through the current tiers). `window()` reads the finest tier
  covering the X window and re-bins it to ≤ `RPS_TARGET_POINTS` = 300
  points (bin averages, bin-center timestamps, empty bins skipped so gaps
  stay gaps) — point density follows whatever rate data actually arrives:
  with the default 5 s backend tick a 10-minute window tops out at ~120
  points, with a faster tick it reaches 300. Since 2026-08-18 the same
  archive also samples each name_id series (keys `name:<id>@<app>`),
  feeding the "Show details" breakdown; name history starts collecting
  from first deployment of this feature and, like app history, covers only
  times the dashboard was open (x-axis is real time, gaps compress, no
  interpolation). `/metrics` is therefore polled on EVERY tab (views still
  render only on their own route) so the archive keeps collecting.
- Clients: `renderClients()` builds the shell once; `renderClientsData(m)`
  per poll does in-place `[data-role=...]` updates and rebuilds only the
  limits + cursors tables. `mergedApps(m)` = live + file-only apps. Perms
  drift detection: `m.config.perms_version !== permsData.version` →
  `loadPermsData()`. Expansion via a `clientsExpanded` Set; detached scopes
  (`detachApps`/`detachNames`) persist only when they carry content.
  Weight chip → `openWeightPop` popover (0.1–10 step 0.1, auto-POST
  /app_weight on release); `w-alt` accent when ≠ 1.
  Tree rows align like a table via FIXED-WIDTH meta slots
  (`.tm-weight`/`.weight-label` 50px, `.tm-spark` 70px, `.tm-rps` 11ch,
  `.tm-limit` 10ch, `.tm-p50` 11ch, `.tm-seen` 9ch, `.blockbtn` min-width
  70px): app rows carry weight+limit, name rows carry `seen` + spacer
  slots (`.tm-weight`, `.tm-limit`) where apps have real content — the name
  sparkline lives INSIDE `.tree-meta` (first position), not left of it.
- Permission editor badge model: 6 badges per row (5 verbs + INDEX), click cycles allow → deny
  → inherit (explicit SOLID / inherited DASHED / none HOLLOW; collections
  inherit-db GRAY FILL). Collections: caret expands a db row → real
  collections + overrides + "+ add". Globs: own badges + ↺ + ✕; ACTIVE globs
  lock matching rows + 🔒. Save: `queuePermsSave()` chain → POST /perms →
  GET /perms → `rebuildOpenPanels()`. Search clears after save (pre-existing).
- Config tab form: field spec `groups: [name, hint|null, CfgField[]][]`;
  `CfgField{path,label,kind,min?,max?,step?,unit?,prefix?,options?}`; kinds
  range/text/select. 2 groups side by side
  (`.config-grid` auto-fit): General (permission_file text, JWT lifetime,
  per-IP auth, session TTL, health TTL, log_level), Rate limiting
  (multiplier, target p50, sensitivities, growth, min/max docs, tick, ema).
  Save flow: `#cfg-save` onclick →
  `structuredClone(configData.config)` → collect
  `form.querySelectorAll("input[data-path], select[data-path]")` →
  POST `/config` → re-render + snack. `renderConfigForm` rebuilds ALL sliders
  from `configData.config` on every load/save; dirty state via module-level
  `configDirty` + `markCfgDirty()`; failed save keeps dirty + re-enables
  Save. Dragging a slider back to its original value still counts as dirty
  (no baseline diff) — accepted. Per-browser display prefs live in
  localStorage, NOT server config: theme (`localStorage["xdb-theme"]`,
  default dark; `#theme-btn` in the topbar toggles it — sun/moon icons +
  Light/Dark label), chart smoothing coefficient (`xdb-smoothing-alpha`,
  default 0.6, slider 0–1 step 0.01, 0 = raw) and metrics poll interval
  (`xdb-poll`, default 2 s, slider 0.1–10). The last two are edited in the
  `#settings-btn` popover (gear + "Settings" topbar button; `settings-pop`
  CSS, outside-click closes). Chart smoothing is a TensorBoard-style EMA
  (`emaSmooth` in charts.ts — their `resmoothDataset`: 1st-order IIR low-pass
  + debias division, α clamped to 0.99) computed AT DRAW TIME over the raw
  series: the overview mini charts keep raw history in `systemHistory`
  (state.ts) and the RPS chart smooths each plotted series/stack row in
  `updateRpsChart` (linear filter → smoothed cum rows ≡ cumulated smoothed
  bands, so stacks still sum to the smoothed app line) — moving the slider
  re-smooths the whole visible history instantly (state.ts
  `setChartSmoothing` re-renders the overview; sparklines stay raw); poll
  changes call `restartPolling()` immediately. Accessors in state.ts:
  get/setChartSmoothing, get/setPollInterval.
  Blocked identifiers card = full-width below
  the columns.
- Logs tab: file-backed store (see config-world.md + api.md logs endpoint).
  `.logs-head` h3 + [↻ Refresh] [⬇ Download] top-right; `.logs-filterbar`
  (position:relative) = "Add filter" + `#logs-fbadges` + retention line +
  `.logs-pop` popover NESTED INSIDE the filterbar (must stay a CHILD of the
  positioned anchor; the outside-close handler must use the CLASS selector —
  it once used an ID selector on a class-only element and every mousedown
  closed the popover). `.logs-box` flex-fills the card
  (`calc(100vh - 64px - 48px - 14px)`). Multi-value filters:
  `logFilters{levels[], loggers[], apps[], names[], regex}` — OR within a
  category, AND across; badges per category (`.f-group` + `.f-chip`);
  popover add-flow with typeahead + suggestion chips; name picker from
  (app,name) facets. Paging: `LOG_PAGE = 300`; scroll near top (40px) →
  `?limit=300&before=<oldest loaded seq>`; `logNoMore` stop. AUTO-PAGING
  (`ensureMatches`): after every render, while fewer than LOG_PAGE rows are
  visible the tab keeps pulling older pages (same `logMatches` predicate —
  works for any composed filter) up to `MAX_AUTO_PAGES = 40` (12k lines),
  because matching lines may sit behind a DEBUG-flooded newest window;
  `#logs-status` (muted line above the box) shows "searching older logs…"
  progress and the end states ("no lines match…" / scan-cap reached).
  `fetchOlder` is module-level and returns the count of rows added (-1 =
  couldn't run; 0 = page empty → `logNoMore`). MEMORY BOUND: the client
  retains at most `LOG_MAX = LOG_PAGE × (MAX_AUTO_PAGES + 2)` = 12,600 rows
  in `logLoaded` + the DOM. `pruneRetained()` runs at the end of every
  `fetchOlder()` and, once the cap is crossed, drops the OLDEST excess rows
  (updating `logOldestSeq`/`logNoMore`) and re-renders `renderLogList()` —
  and LATCHES `logCapped`, which makes `fetchOlder` refuse further older
  paging until the next fresh base load (2026-08-22 fix: without it the
  prune raised `logOldestSeq` so the next fetch re-fetched the just-evicted
  page, pruned again, re-rendered the 12k-row DOM — a fetch→prune→rerender
  LIVELOCK that kept churning on every subsequent search/scroll). Old
  pages live in the rotated on-disk files and are re-fetchable, so capping
  the client ring only bounds memory (this was added for an unbounded-RAM
  report: repeated searches used to accumulate every pulled page forever).
  MEMORY RELEASE: the router (state.ts) exposes a generic per-view `leaves`
  hook ({"logs": releaseLogs}) called before the incoming view renders, so
  leaving the Logs tab drops `logLoaded` + facet lists + `logTotal` (and bumps
  `logGen`); `logFilters` is deliberately
  KEPT, so leaving/returning restores the user's active filtered view and
  re-runs the auto-search. `logGen` (bumped by every fresh base load and by
  release) is captured by each in-flight `fetchOlder()` and checked after its
  await, so a stale older page resolved after a clear/leave/reload is
  discarded instead of spliced onto the freshly cleared ring. Clear-all calls
  the same `load()` used on tab entry (~ just switched to Logs): newest
  300 unfiltered.
  Download = `/logs` no params → raws → blob.
- Design system (styles.css): tokens in three blocks — `:root` (light),
  `@media (prefers-color-scheme: dark) :root:not([data-theme="light"])`,
  `:root[data-theme="dark"]` (forced). Primary #6d4aff. `.config-grid` =
  `repeat(auto-fit, minmax(300px, 1fr))`. Badges: `.badge` pill +
  `.ok`/`.bad`/`.warn`/`.info`/`.primary` variants; `.hidden {
  display:none !important }`. `.card h3 { display:flex; align-items:center;
  gap:8px }` — right-edge pinning via `margin-left:auto` on a child works
  there reliably (see dirty-pill placement above).
- Per-request DEBUG log lines only when `dashboard.log_level = "debug"`
  (hot-reloadable via `reload::Layer` + `with_filter`; hook applied at config
  save/reload/watcher). The battery runs at info → ~0 DEBUG lines by design.

### Request log line formats (2026-08-18: peer addr added)

Every identity-carrying log line ends with the effective client address
  (`from IP` or `from IP:PORT`):
  - `network.trust_proxy_headers` ON (compose/prod): the proxy header IP
    (`X-Real-IP`, else last XFF entry) — no port, the proxy doesn't forward
    one; verified live 2026-08-18.
  - OFF (bare metal default): the socket peer `IP:PORT`. Behind a Docker
    port-forward that's the bridge gateway (172.x.0.1), not the real client
    IP. Behind compose+nginx in prod, the flag is ON via the
    `TRUST_PROXY_HEADERS=true` env in compose.yaml (safe: port published to
    127.0.0.1 only).
- `/q` + `/ls` debug trace (routes_q): `GET /q/db/coll from 127.0.0.1:55555 as name@app`
  — identity stays LAST so `log_identify`'s `" as "` split keeps working.
- `/auth` login lines (routes_misc, INFO/WARN): `login OK: name@app from 127.0.0.1:55555`
  (also failed/blocked; `login throttled: IP:PORT`).
- `state.rs log_identify` strips the trailing `" from <addr>"` on login lines
  before extracting the app facet — legacy lines without an addr still parse
  (unit test `log_identify_formats`).
