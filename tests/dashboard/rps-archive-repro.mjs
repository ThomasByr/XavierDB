// Verification for the RPS archive subsampling change (2026-08-24):
//   1. dense sampling (1 s cadence, fast backend tick) → a 10-minute window
//      reaches ~RPS_TARGET_POINTS (300) points
//   2. sparse sampling (5 s cadence, default backend tick) → same window is
//      data-capped below target (no fake upsampling)
//   3. every selectable window resolves to a covering tier with sane counts
//   4. a v1 localStorage archive is discarded (key removed, history starts
//      blank under the v2 key)
// Bundles ts/rps-archive.ts with esbuild and runs it against stubbed
// localStorage/window — no jsdom needed (the module touches no DOM).
// Run from the repo root: node tests/dashboard/rps-archive-repro.mjs
import { build } from "esbuild";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const REPO = process.env.XDB_REPO || fileURLToPath(new URL("../..", import.meta.url));
const out = join(mkdtempSync(join(tmpdir(), "xdb-arch-")), "arch.mjs");
await build({
  entryPoints: [join(REPO, "src/assets/ts/rps-archive.ts")],
  bundle: true,
  format: "esm",
  outfile: out,
  target: "es2020",
});

const now = 1_800_000_000; // fixed epoch secs
const load = (store) => {
  // fresh module instance per load (module-level rpsArchive singleton)
  globalThis.localStorage = {
    getItem: (k) => (store.has(k) ? store.get(k) : null),
    setItem: (k, v) => store.set(k, String(v)),
    removeItem: (k) => store.delete(k),
  };
  globalThis.window = { addEventListener: () => {} };
  return import("file://" + out.replace(/\\/g, "/") + "?v=" + Math.random());
};

const fail = (msg) => {
  console.log("FAIL: " + msg);
  process.exit(1);
};

// --- 1. dense sampling (1 s cadence, "fast backend tick") --------------------
{
  const { rpsArchive, RPS_TARGET_POINTS } = await load(new Map());
  for (let t = 0; t < 600; t++)
    rpsArchive.sample([{ app: "a", rps: 10 + Math.sin(t / 20) }], (now - 600 + t) * 1000 + 500);
  const pts = rpsArchive.window(["a"], 600, now).get("a");
  console.log("10 min @1s samples  ->", pts.length, "points (target " + RPS_TARGET_POINTS + ")");
  if (pts.length > RPS_TARGET_POINTS) fail("exceeds target");
  if (pts.length < 290) fail("dense data should reach ~target");
}

// --- 2. sparse sampling (5 s cadence, current backend tick) ------------------
{
  const { rpsArchive } = await load(new Map());
  for (let t = 0; t < 600; t += 5)
    rpsArchive.sample([{ app: "a", rps: 5 + Math.sin(t / 30) }], (now - 600 + t) * 1000);
  const pts = rpsArchive.window(["a"], 600, now).get("a");
  console.log("10 min @5s samples  ->", pts.length, "points (data-capped)");
  if (pts.length < 100 || pts.length > 125) fail("expected ~120 points");
}

// --- 3. every selectable window: covering tier + sane point count ------------
{
  const { rpsArchive, RPS_WINDOWS, RPS_TIERS } = await load(new Map());
  for (let t = 0; t < 3600; t += 2)
    rpsArchive.sample([{ app: "a", rps: 5 + Math.sin(t / 100) }], (now - 3600 + t) * 1000);
  console.log("windows (1 h of @2s samples):");
  for (const [label, win] of RPS_WINDOWS) {
    const p = rpsArchive.window(["a"], win, now).get("a");
    const ti = Math.max(0, RPS_TIERS.findIndex(([, keep]) => keep >= win));
    console.log(
      `  ${label.padEnd(11)} tier=${String(RPS_TIERS[ti][0] + "s").padEnd(7)} pts=${p.length}`,
    );
    if (win <= 3600 && p.length < 30) fail(label + ": too few points for in-range window");
  }
}

// --- 4. v1 archive is discarded, not migrated ---------------------------------
{
  const v1 = {
    firstT: now - 3600,
    series: {
      old: {
        lastT: now - 10,
        tiers: [
          { ts: [now - 600, now - 590, now - 580], vs: [1, 2, 3], open: { t: now - 570, sum: 8, n: 2 } },
          { ts: [now - 3600, now - 1800], vs: [4, 5], open: null },
          { ts: [], vs: [], open: null },
          { ts: [], vs: [], open: null },
          { ts: [], vs: [], open: null },
          { ts: [], vs: [], open: null },
        ],
      },
    },
  };
  const store = new Map([["xdb-rps-archive-v1", JSON.stringify(v1)]]);
  const { rpsArchive } = await load(store);
  const pts = rpsArchive.window(["old"], 3600, now).get("old");
  rpsArchive.flushIfDirty(); // no save expected: nothing dirty (nothing migrated)
  console.log("v1 discard         ->", pts.length, "points, firstT =", rpsArchive.startSec);
  if (pts.length !== 0) fail("v1 points leaked into the blank archive");
  if (rpsArchive.startSec !== 0) fail("firstT should stay 0 (blank archive)");
  if (store.has("xdb-rps-archive-v2")) fail("v2 written on load");
  if (store.has("xdb-rps-archive-v1")) fail("v1 not removed");
  // a fresh sample afterwards starts a clean v2 archive
  rpsArchive.sample([{ app: "a", rps: 1 }], now * 1000);
  rpsArchive.flushIfDirty();
  if (!store.has("xdb-rps-archive-v2")) fail("fresh v2 not written after sampling");
  if (rpsArchive.window(["old"], 3600, now).get("old").length !== 0) fail("old v1 series resurrected");
}

console.log("\nALL CHECKS PASSED");
