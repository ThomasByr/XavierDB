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
  the RPS archive keeps sampling; the sample call also carries per-name
  rps). NOTE: deliberate import cycle
  state ↔ view-* (safe: cross-uses happen at call time, never at module
  evaluation).
  - `rps-archive.ts` — tiered long-window RPS history (localStorage); stores
  app series AND `name:<id>@<app>` series (feeds the "Show details"
  stacked breakdown) under the same map + localStorage
  `xdb-rps-archive-v1`
  - `charts.ts` — sparkline, drawMini, getCss, lineColor palette, withAlpha
  - `mongo.ts` — topbar MongoDB status widget
  - `view-overview.ts` / `view-clients.ts` / `view-config.ts` / `view-logs.ts`
  — one module per tab (overview holds the all-apps RPS chart + window
  popover + "Show details" name_id breakdown popover + hover
  crosshair/tooltip — see knowledge/architecture.md "Overview")
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
edits and after Save. To test other views, extend the stub routes. For
CANVAS rendering (chart draw calls), stub
`HTMLCanvasElement.prototype.getContext` with a recording Proxy that logs
every method call + property set (jsdom has no canvas) and stub
`measureText` to `{width: len*5.5}`; see `details-repro.mjs` (asserts stack
line alphas, band fills, name labels, crosshair dashes, tooltip rows;
pins the name_id threshold to 0% via localStorage before eval) and
`threshold-repro.mjs` (2026-08-20: threshold slider INSIDE the details
popover — no standalone button; others-band merge/hatch, 0.50→0.10 band
ramp, 0%/high-threshold extremes, live slider re-render, full hover
detail; check 5a was fixed 2026-08-21 to match the documented 2026-08-20
behavior — the tooltip lists DRAWN bands only) and `focus-repro.mjs`
(2026-08-21: Global ⇄ Focus segmented switch in the RPS card header, ▾
app-picker popover, focus legend/summary/title, Show-details disabled in
Focus, localStorage persistence + restore).
Harness copies live in the dev machine's temp dir (see
`.pi/notes/credentials.md`).

## UI gotchas (architecture context)

- Config tab save is EXPLICIT — slider edits alone don't persist; a reload
  discards them (dirty pill + disabled Save while clean; see
  knowledge/architecture.md Dashboard).
- Any new theme-aware CSS token must land in ALL THREE theme blocks
  (`:root`, dark media query, `[data-theme="dark"]`).
