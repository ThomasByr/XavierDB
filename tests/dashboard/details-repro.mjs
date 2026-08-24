// Verification for the "Show details" stacked name_id breakdown on the
// Overview "All apps · RPS" chart:
//  1. "Show details" button (top right, below the summary) + popover with
//     per-app toggle switches (multi-select, persisted)
//  2. Toggling an app draws a stacked breakdown: bands + cumulative lines
//     (biggest contributor at the bottom, least transparent; top level NOT
//     stroked — the app line tops the stack), name_id labels at the right
//  3. Hover: dashed crosshair + light tooltip panel (app rows + nested
//     name_id rows)
// Drives the repo bundle directly (same bytes as the compile-time embed).
import { JSDOM } from "jsdom";
import { readFileSync } from "fs";
import { fileURLToPath } from "node:url";

const REPO = process.env.XDB_REPO || fileURLToPath(new URL("../..", import.meta.url));
const html = readFileSync(REPO + "/src/assets/index.html", "utf8");
const appjs = readFileSync(REPO + "/src/assets/app.js", "utf8");

const mkMetrics = () => ({
  ts: Date.now(),
  qps: 40,
  config: { poll_seconds: 2, theme: "system", graph_smoothing: 5, cfg_version: 0, perms_version: 0, health_ttl_seconds: 5, multiplier: 1 },
  system: { cpu_pct: 5, mem_pct: 40, mem_used_mb: 100, mem_total_mb: 256, disk_pct: 30, disk_used_mb: 10, disk_total_mb: 50, net_rx_kbps: 1, net_tx_kbps: 1, uptime_s: 100, ts_ms: Date.now() },
  health: { status: "ok", mongodb: { ping_latency_ms: 1 }, app: { total_requests: 500 } },
  apps: [
    { app: "alpha", blocked: false, weight: 1, rps: 50, p50_ms: 3, limit: 100, rps_history: [1, 2, 3], names: [
      { name: "a1", id: "a1@alpha", blocked: false, rps: 50, p50_ms: 3, total_requests: 5, last_seen_ms: Date.now() - 30000, rps_history: [1, 2] },
    ] },
    { app: "beta", blocked: false, weight: 2, rps: 40, p50_ms: 12, limit: null, rps_history: [3, 2, 1], names: [
      { name: "n1", id: "n1@beta", blocked: false, rps: 10, p50_ms: 12, total_requests: 5, last_seen_ms: Date.now() - 30000, rps_history: [1, 2] },
      { name: "n2", id: "n2@beta", blocked: false, rps: 30, p50_ms: 8, total_requests: 9, last_seen_ms: Date.now() - 30000, rps_history: [2, 3] },
    ] },
  ],
  cursors: { count: 0, list: [] },
});

const vc = new (await import("jsdom")).VirtualConsole();
vc.on("jsdomError", (e) => console.log("[jsdomError]", e.message));
const dom = new JSDOM(html, { url: "http://127.0.0.1:8000/dashboard/", runScripts: "outside-only", pretendToBeVisual: true, virtualConsole: vc });
const { window } = dom;
const { document } = window;

window.matchMedia = () => ({ matches: false, addEventListener() {}, addListener() {} });
window.structuredClone = (x) => JSON.parse(JSON.stringify(x));

// recording 2d context — every method call and property set is captured
let calls = [];
const ctxStub = new Proxy({}, {
  get: (t, k) => {
    if (k === "canvas") return {};
    if (k === "measureText") return (text) => ({ width: String(text).length * 5.5 });
    return (...args) => { calls.push([k, ...args]); };
  },
  set: (t, k, v) => { calls.push(["set:" + k, v]); return true; },
});
window.HTMLCanvasElement.prototype.getContext = () => ctxStub;

window.fetch = async (url) => {
  const u = String(url);
  if (u.includes("/dashboard/api/session")) return { ok: true, status: 200, json: async () => ({ username: "admin" }) };
  if (u.includes("/dashboard/api/metrics")) return { ok: true, status: 200, json: async () => mkMetrics() };
  if (u.includes("/dashboard/api/perms")) return { ok: true, status: 200, json: async () => ({ version: 1, apps: [] }) };
  if (u.includes("/dashboard/api/databases")) return { ok: true, status: 200, json: async () => ({ databases: [] }) };
  return { ok: true, status: 200, json: async () => ({ ok: true }) };
};

// 2026-08-20: name_id threshold defaults to 33% (smaller contributors
// merge into a hatched "others" band — see threshold-repro.mjs). This
// repro predates it and tests the full-detail breakdown: pin the
// threshold to 0 BEFORE the bundle evaluates (it reads localStorage at
// module init).
window.localStorage.setItem("xdb-rps-threshold", "0");
window.eval(appjs);
window.dispatchEvent(new window.Event("load"));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
let fails = 0;
const check = (name, cond, extra = "") => { console.log((cond ? "PASS" : "FAIL") + ": " + name + (extra ? "  [" + extra + "]" : "")); if (!cond) fails++; };
// helpers over the recorded calls
const setVals = (prop) => calls.filter((c) => c[0] === "set:" + prop).map((c) => c[1]);
const methodCalls = (m) => calls.filter((c) => c[0] === m);

await wait(300);

// ---------- 1. Show details button + popover ----------
const dbtn = document.querySelector("#ov-rps-details");
check("1a: 'Show details' button present in .rps-head next to the legend", !!dbtn && !!dbtn.closest(".rps-head") && !!document.querySelector("#ov-rps-legend").closest(".rps-head"), "");
check("1b: button label starts plain", dbtn.textContent === "Show details", dbtn.textContent);

dbtn.click();
await wait(20);
const pop = document.querySelector(".det-pop");
check("1c: click opens the details popover", !!pop, "");
const rows = pop ? [...pop.querySelectorAll(".dp-row")] : [];
check("1d: one toggle row per app (alpha, beta)", rows.length === 2 && rows.map((r) => r.querySelector(".dp-name").textContent).join(",") === "alpha,beta", String(rows.length));
const betaRow = rows.find((r) => r.querySelector(".dp-name").textContent === "beta");
check("1e: rows are switches (checkbox) with app color swatch", !!betaRow.querySelector("input[type=checkbox]") && !!betaRow.querySelector(".dp-sw"), "");

// ---------- 2. toggle beta -> stacked breakdown ----------
const betaCb = betaRow.querySelector("input[type=checkbox]");
betaCb.checked = true;
betaCb.dispatchEvent(new window.Event("change", { bubbles: true }));
await wait(20);
check("2a: button label shows the selection count", dbtn.textContent === "Show details · 1", dbtn.textContent);
check("2b: selection persisted to localStorage", window.localStorage.getItem("xdb-rps-details") === JSON.stringify(["beta"]), window.localStorage.getItem("xdb-rps-details"));

// draw calls from the last updateRpsChart: isolate by clearing before re-poll
calls = [];
await wait(2100); // one more poll (2s) redraws the chart with the stack
const strokes = setVals("strokeStyle");
const fills = setVals("fillStyle");
// beta color = lineColor("beta") — read the raw style attribute (jsdom
// normalizes style.background to rgb() notation)
const legendSwatches = [...document.querySelectorAll("#ov-rps-legend .rl i")].map((i) =>
  (i.getAttribute("style") || "").match(/#[0-9a-fA-F]{6}/)?.[0] ?? "");
const betaHex = legendSwatches[1];
const betaRgba = (a) => {
  const h = betaHex.replace("#", "");
  return `rgba(${parseInt(h.slice(0, 2), 16)},${parseInt(h.slice(2, 4), 16)},${parseInt(h.slice(4, 6), 16)},${a})`;
};
check("2c: stacked line stroked at alpha 0.85 (biggest contributor)", strokes.includes(betaRgba(0.85)), JSON.stringify(strokes));
check("2d: band fills at alpha 0.50 and 0.10 (graded by contribution)", fills.includes(betaRgba(0.5)) && fills.includes(betaRgba(0.1)), "");
check("2e: NO alpha-graded top line (top level = app line, not stroked separately)", !strokes.includes(betaRgba(0.5)), "");
const texts = methodCalls("fillText").map((c) => String(c[1]));
check("2f: name_id labels drawn at the right edge (n2, n1)", texts.includes("n2") && texts.includes("n1"), JSON.stringify(texts));

// ---------- 3. second app toggle (multi-select) ----------
const alphaRow = rows.find((r) => r.querySelector(".dp-name").textContent === "alpha");
const alphaCb = alphaRow.querySelector("input[type=checkbox]");
alphaCb.checked = true;
alphaCb.dispatchEvent(new window.Event("change", { bubbles: true }));
await wait(20);
check("3a: both apps selected, count = 2", dbtn.textContent === "Show details · 2", dbtn.textContent);
// alpha has a single name -> 0 extra stroked lines, 1 band, 1 label
calls = [];
await wait(2100);
const alphaTexts = methodCalls("fillText").map((c) => String(c[1]));
check("3b: alpha's name label also drawn (a1)", alphaTexts.includes("a1"), JSON.stringify(alphaTexts));

// ---------- 4. hover crosshair + tooltip ----------
const canvas = document.querySelector("#ov-rps-canvas");
canvas.getBoundingClientRect = () => ({ left: 0, top: 0, width: 600, height: 190, right: 600, bottom: 190, x: 0, y: 0 });
calls = [];
canvas.dispatchEvent(new window.MouseEvent("mousemove", { clientX: 300, clientY: 60 }));
await wait(20);
check("4a: dashed vertical crosshair drawn", methodCalls("setLineDash").length >= 1, JSON.stringify(methodCalls("setLineDash").map((c) => c[1])));
check("4b: tooltip panel drawn (semi-transparent surface fill)", fills2().some((v) => String(v).startsWith("rgba(") && String(v).endsWith("0.94)")), JSON.stringify(fills2()));
function fills2() { return setVals("fillStyle"); }
const ttTexts = calls.filter((c) => c[0] === "fillText").map((c) => String(c[1]));
check("4c: tooltip lists apps + nested name_ids", ["alpha", "a1", "beta", "n1", "n2"].every((t) => ttTexts.includes(t)), JSON.stringify(ttTexts));
check("4d: tooltip shows time header + rps values", ttTexts.some((t) => /:/.test(t)) && ttTexts.some((t) => /^[\d.]+$/.test(t)), "");
canvas.dispatchEvent(new window.MouseEvent("mouseleave"));
await wait(20);
calls = [];
await wait(2100);
check("4e: crosshair cleared after mouseleave", methodCalls("setLineDash").length === 0, "");

// ---------- 5. per-name archive sampling ----------
window.dispatchEvent(new window.Event("beforeunload")); // flushIfDirty -> save
const arch = JSON.parse(window.localStorage.getItem("xdb-rps-archive-v2") || "{}");
check("5a: name_id series archived (name:n1@beta, name:a1@alpha)", !!arch.series && !!arch.series["name:n1@beta"] && !!arch.series["name:a1@alpha"] && !!arch.series["beta"], Object.keys(arch.series || {}).join(","));

// ---------- 6. popover interactions ----------
document.body.dispatchEvent(new window.MouseEvent("mousedown", { bubbles: true })); // close any open pop
await wait(20);
dbtn.click();
await wait(20);
const pop2 = document.querySelector(".det-pop");
check("6a: popover re-opens after outside-click close", !!pop2, "");
if (pop2) {
  document.body.dispatchEvent(new window.MouseEvent("mousedown", { bubbles: true }));
  await wait(20);
  check("6b: outside mousedown closes the popover", !document.querySelector(".det-pop"), "");
}
const sel = JSON.parse(window.localStorage.getItem("xdb-rps-details") || "[]").sort();
check("6c: localStorage holds both selections", JSON.stringify(sel) === JSON.stringify(["alpha", "beta"]), window.localStorage.getItem("xdb-rps-details"));

console.log(fails ? `\n${fails} FAILURE(S)` : "\nALL PASS");
process.exit(fails ? 1 : 0);
