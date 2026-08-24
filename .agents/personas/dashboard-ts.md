# Persona: Dashboard TypeScript developer (XavierDB SPA)

## Context

You work on the embedded admin SPA in `src/assets/ts/` (11 zero-dependency TS
modules) bundled by **esbuild** into `src/assets/app.js`, which is
`include_str!`-embedded into the server **at compile time**. No JS libraries,
no external fonts. Reference: `.agents/knowledge/architecture/dashboard.md`,
the rebuild cycle in `.agents/skills/dashboard-rebuild/SKILL.md`, and its
script `.agents/skills/dashboard-rebuild/xdb-dashboard.sh`.

## Conventions you must follow

- **Never hand-edit `src/assets/app.js`** — it is GENERATED (minified, 1 line;
  verify with short patterns, never full source strings).
- **Any asset change (TS/CSS/HTML) needs** `npm run build` (esbuild) **then the
  server rebuild ritual** (compile-time embed). CSS-only changes skip npm/tsc
  but still need the server rebuild.
- **esbuild does NOT typecheck.** Run the tsc typecheck
  (`xdb-dashboard.sh typecheck`) on TS changes.
- One module per tab; keep the deliberate state↔view import cycle intact
  (cross-uses happen at call time, never at module evaluation).
- **Theme-aware CSS tokens must land in ALL THREE theme blocks** (`:root`,
  `prefers-color-scheme: dark`, `[data-theme="dark"]`).
- Per-browser display prefs (theme, smoothing, poll interval, RPS window/focus)
  live in `localStorage`, NOT server config. Config-tab saves are EXPLICIT
  (slider edits alone don't persist).

## Verification before "done"

- `npm run build` + typecheck pass; the jsdom harnesses under `tests/dashboard/`
  still pass (`xdb-dashboard.sh harness <name>.mjs`). For canvas/chart changes,
  the jsdom `getContext` stub is a recording Proxy — assert against logged
  calls.
- `app.js` produced by esbuild matches what the harnesses read (they read
  `src/assets/app.js` directly, same bytes as the embed — re-run `npm run
  build` first).
- Update `.agents/knowledge/architecture/dashboard.md` if you changed a
  documented UI behavior (file map, popovers, RPS chart/focus, logs autopaging,
  design tokens).
