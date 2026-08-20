# Dashboard rebuild cycle (compile-time embed — 2 steps!)

The SPA is embedded via `include_str!` AT COMPILE TIME. **ANY asset change —
TS, CSS, HTML — needs: kill server → `cargo build --tests` → restart.**
CSS-only changes skip npm/tsc but still need the server rebuild. The bundle
build (esbuild) and the embed (rustc) are TWO separate steps.

## File map

- `src/assets/ts/` — the SOURCE (TypeScript modules, zero deps; esbuild
  bundles them). Do NOT edit `src/assets/app.js` by hand — it is GENERATED
  (minified, 1 line — verify with SHORT patterns, never full source strings).
  - `app.ts` — entry point only (theme, topbar shell, login boot)
  - `core.ts` — `$`/`el`/`esc`/fmt* helpers, snackbar, confirm dialog
  - `state.ts` — `api()` fetch wrapper, Metrics/AppNode/ClientNode types,
  `lastMetrics`, login view, hash router, poll loop (polls on EVERY tab so
  the RPS archive keeps sampling). NOTE: deliberate import cycle
  state ↔ view-* (safe: cross-uses happen at call time, never at module
  evaluation).
  - `rps-archive.ts` — tiered long-window RPS history (localStorage)
  - `charts.ts` — sparkline, drawMini, getCss, lineColor palette
  - `mongo.ts` — topbar MongoDB status widget
  - `view-overview.ts` / `view-clients.ts` / `view-config.ts` / `view-logs.ts`
  — one module per tab (overview holds the all-apps RPS chart + window
  popover)
  - `perm-widget.ts` — permission editor + effective rules (driven by
  view-clients; receives the live db list via `PermCtx.dbs` to stay
  cycle-free)
- `src/assets/index.html` — static shell; `src/assets/styles.css` — design
  tokens + styles.
- `src/assets.rs` (Rust) — serves the three files under `/dashboard/`
  no-cache.

## Rebuild cycle

```bash
npm run build     # esbuild ts/app.ts -> src/assets/app.js (skip if CSS/HTML-only)
# typecheck (esbuild does NOT typecheck):
#   npx --yes -p typescript tsc --noEmit --strict --target es2020 --lib es2020,dom src/assets/ts/app.ts
# then the server ritual: kill -> cargo build --tests -> start (restart-ritual.md)
```

## Browser-behavior debugging without a browser (jsdom harness)

A jsdom repro drives the SERVED bundle: fetch `/dashboard/` index.html +
app.js from the running server (**MUST re-fetch after every server rebuild —
the embed is compile-time**), stub fetch/matchMedia, simulate clicks, and
assert on the DOM after input/change events, Save clicks, and simulated
route re-entry. The config-tab repro pattern: mutable serverConfig + echo
POST response + `postBodies` capture; asserts slider value + readout after
edits and after Save. To test other views, extend the stub routes. Harness
copies live in the dev machine's temp dir (see `.pi/notes/credentials.md`).

## UI gotchas (architecture context)

- Config tab save is EXPLICIT — slider edits alone don't persist; a reload
  discards them (dirty pill + disabled Save while clean; see
  knowledge/architecture.md Dashboard).
- Any new theme-aware CSS token must land in ALL THREE theme blocks
  (`:root`, dark media query, `[data-theme="dark"]`).
