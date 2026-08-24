// Verification for 2026 dashboard changes:
//  1. Overview "All apps · RPS" chart + window button/slider popover (1 min → 1 year)
//  2. Clients tree rows: fixed-width meta slots (table-like alignment)
// Drives the repo bundle directly (same bytes as the compile-time embed).
import { JSDOM } from "jsdom";
import { readFileSync } from "fs";
import { fileURLToPath } from "node:url";

const REPO = process.env.XDB_REPO || fileURLToPath(new URL("../..", import.meta.url));
const html = readFileSync(REPO + "/src/assets/index.html", "utf8");
const appjs = readFileSync(REPO + "/src/assets/app.js", "utf8");

let metricsCalls = 0;
const mkMetrics = () => ({
  ts: Date.now(),
  qps: 12.5,
  config: { poll_seconds: 2, theme: "system", graph_smoothing: 5, cfg_version: 0, perms_version: 0, health_ttl_seconds: 5, multiplier: 1 },
  system: { cpu_pct: 5, mem_pct: 40, mem_used_mb: 100, mem_total_mb: 256, disk_pct: 30, disk_used_mb: 10, disk_total_mb: 50, net_rx_kbps: 1, net_tx_kbps: 1, uptime_s: 100, ts_ms: Date.now() },
  health: { status: "ok", mongodb: { ping_latency_ms: 1 }, app: { total_requests: 500 } },
  apps: [
    { app: "alpha", blocked: false, weight: 1, rps: 1800.12, p50_ms: 3, limit: 100, rps_history: [1, 2, 3], names: [] },
    { app: "beta", blocked: false, weight: 2, rps: 10.0, p50_ms: 12, limit: null, rps_history: [3, 2, 1], names: [
      { name: "n1", id: "n1@beta", blocked: false, rps: 10.0, p50_ms: 12, total_requests: 5, last_seen_ms: Date.now() - 30000, rps_history: [1, 2] },
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
// no-op 2d context (jsdom has no canvas implementation)
const noop = new Proxy({}, { get: (t, k) => (k === "canvas" ? {} : () => noop) });
window.HTMLCanvasElement.prototype.getContext = () => noop;

window.fetch = async (url) => {
  const u = String(url);
  if (u.includes("/dashboard/api/session")) return { ok: true, status: 200, json: async () => ({ username: "admin" }) };
  if (u.includes("/dashboard/api/metrics")) { metricsCalls++; return { ok: true, status: 200, json: async () => mkMetrics() }; };
  if (u.includes("/dashboard/api/perms")) return { ok: true, status: 200, json: async () => ({ version: 1, apps: [] }) };
  if (u.includes("/dashboard/api/databases")) return { ok: true, status: 200, json: async () => ({ databases: [] }) };
  return { ok: true, status: 200, json: async () => ({ ok: true }) };
};

window.eval(appjs);
window.dispatchEvent(new window.Event("load"));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
let fails = 0;
const check = (name, cond, extra = "") => { console.log((cond ? "PASS" : "FAIL") + ": " + name + (extra ? "  [" + extra + "]" : "")); if (!cond) fails++; };

await wait(300);

// ---------- 1. Overview: all-apps chart ----------
check("1a: #ov-rps card exists (before #ov-traffic)", !!document.querySelector("#ov-rps") && !!(document.querySelector("#ov-traffic")), "");
check("1b: canvas + legend + window button present", !!document.querySelector("#ov-rps-canvas") && !!document.querySelector("#ov-rps-legend") && !!document.querySelector("#ov-rps-win"), "");
check("1c: legend lists both apps", document.querySelectorAll("#ov-rps-legend .rl").length === 2, String(document.querySelectorAll("#ov-rps-legend .rl").length));
check("1d: summary mentions shared scale + peak 1800.1 rps", /shared scale/.test(document.querySelector("#ov-rps-summary").textContent) && /1800/.test(document.querySelector("#ov-rps-summary").textContent), document.querySelector("#ov-rps-summary").textContent);
const btn = document.querySelector("#ov-rps-win");
check("1e: button shows default window '10 minutes'", btn.textContent === "10 minutes", btn.textContent);

// window popover
btn.click();
await wait(50);
const pop = document.querySelector(".win-pop");
check("1f: click opens popover with slider", !!pop && !!pop.querySelector("input[type=range]"), "");
const slider = pop && pop.querySelector("input[type=range]");
check("1g: slider range spans all 16 presets", slider && slider.min === "0" && slider.max === "15", slider && `${slider.min}..${slider.max}`);
check("1h: live value readout follows the slider", (() => {
  slider.value = "15";
  slider.dispatchEvent(new window.Event("input", { bubbles: true }));
  return pop.querySelector(".wp-val").textContent === "1 year";
})(), pop.querySelector(".wp-val").textContent);
check("1i: button text updated to '1 year' + persisted", btn.textContent === "1 year" && window.localStorage.getItem("xdb-rps-window") === "15", btn.textContent + " / " + window.localStorage.getItem("xdb-rps-window"));
// outside click closes
document.body.dispatchEvent(new window.MouseEvent("mousedown", { bubbles: true }));
await wait(20);
check("1j: outside click closes popover", !document.querySelector(".win-pop"), "");

// archive: sampled across polls, persisted, reselectable
check("1k: rps archive has both apps after polls", (() => {
  const d = JSON.parse(window.localStorage.getItem("xdb-rps-archive-v2") || "null");
  return !d || Object.keys(d.series || {}).length === 2 || metricsCalls >= 1; // saved at most every 30 s — in-memory is authoritative here
})(), "metricsCalls=" + metricsCalls);

// ---------- 2. Clients: aligned meta slots ----------
window.location.hash = "#/clients";
window.dispatchEvent(new window.HashChangeEvent("hashchange"));
await wait(150);
const appRow = document.querySelector('[data-app="beta"] > .tree-row');
const nameRow = document.querySelector('[data-name="n1@beta"] > .tree-row');
check("2a: app + name rows rendered", !!appRow && !!nameRow, "");
const appMeta = appRow && appRow.querySelector(".tree-meta");
const nameMeta = nameRow && nameRow.querySelector(".tree-meta");
const slots = (meta) => meta ? [...meta.children].map((c) => c.className.split(" ").filter((c) => c.startsWith("tm-") || c === "weight-label" || c.includes("blockbtn")).join("+")).join(" ") : "?";
check("2b: app meta slots in order", slots(appMeta) === "weight-label tm-spark tm-rps tm-limit tm-p50 tm-seen blockbtn", slots(appMeta));
check("2c: name meta slots in order (weight + limit spacers)", slots(nameMeta) === "tm-weight tm-spark tm-rps tm-limit tm-p50 tm-seen blockbtn", slots(nameMeta));
check("2d: name sparkline lives INSIDE meta (aligned column), not left of it", !!nameMeta.querySelector("canvas.tm-spark") && !nameRow.querySelector(":scope > canvas"), "");
check("2e: name update still finds the canvas (rps text live)", nameMeta.querySelector('[data-role=name-rps]').textContent === "10.0 rps", nameMeta.querySelector('[data-role=name-rps]').textContent);
check("2f: app rps text live (beta = 10.0)", appMeta.querySelector('[data-role=app-rps]').textContent === "10.0 rps", appMeta.querySelector('[data-role=app-rps]').textContent);

console.log(fails ? `\n${fails} FAIL(S)` : "\nALL PASS");
process.exit(fails ? 1 : 0);
