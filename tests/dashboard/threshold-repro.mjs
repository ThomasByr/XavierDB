// Verification for the name_id contribution threshold living INSIDE the
// "Show details" popover (2026-08-20, revised: standalone "≥ NN%" button
// removed after feedback):
//  1. NO standalone threshold button in .rps-head; the "Show details"
//     popover carries the threshold section (title + live value + slider
//     0..100 step 1, default 33, persisted)
//  2. Default 33%: name_ids under 33% of the app's window traffic merge
//     into one hatched "others (N)" band (top contributor always kept)
//  3. Band opacity: top band 0.50 fading upward to 0.10 (others hatched)
//  4. 0% shows every band; high thresholds keep only the top contributor
//  5. Hover tooltip still lists EVERY name_id (merged ones included)
// Drives the repo bundle directly (same bytes as the compile-time embed).
import { JSDOM } from "jsdom";
import { readFileSync } from "fs";
import { fileURLToPath } from "node:url";

const REPO = process.env.XDB_REPO || fileURLToPath(new URL("../..", import.meta.url));
const html = readFileSync(REPO + "/src/assets/index.html", "utf8");
const appjs = readFileSync(REPO + "/src/assets/app.js", "utf8");

// app "gamma": 5 names, rps 40/30/10/10/10 (contribs 40%/30%/10%/10%/10%)
const mkMetrics = () => ({
  ts: Date.now(),
  qps: 40,
  config: { poll_seconds: 2, theme: "system", graph_smoothing: 5, cfg_version: 0, perms_version: 0, health_ttl_seconds: 5, multiplier: 1 },
  system: { cpu_pct: 5, mem_pct: 40, mem_used_mb: 100, mem_total_mb: 256, disk_pct: 30, disk_used_mb: 10, disk_total_mb: 50, net_rx_kbps: 1, net_tx_kbps: 1, uptime_s: 100, ts_ms: Date.now() },
  health: { status: "ok", mongodb: { ping_latency_ms: 1 }, app: { total_requests: 500 } },
  apps: [
    { app: "gamma", blocked: false, weight: 1, rps: 100, p50_ms: 3, limit: 100, rps_history: [1, 2, 3], names: [
      { name: "g1", id: "g1@gamma", blocked: false, rps: 40, p50_ms: 3, total_requests: 5, last_seen_ms: Date.now() - 30000, rps_history: [1, 2] },
      { name: "g2", id: "g2@gamma", blocked: false, rps: 30, p50_ms: 3, total_requests: 5, last_seen_ms: Date.now() - 30000, rps_history: [1, 2] },
      { name: "g3", id: "g3@gamma", blocked: false, rps: 10, p50_ms: 3, total_requests: 5, last_seen_ms: Date.now() - 30000, rps_history: [1, 2] },
      { name: "g4", id: "g4@gamma", blocked: false, rps: 10, p50_ms: 3, total_requests: 5, last_seen_ms: Date.now() - 30000, rps_history: [1, 2] },
      { name: "g5", id: "g5@gamma", blocked: false, rps: 10, p50_ms: 3, total_requests: 5, last_seen_ms: Date.now() - 30000, rps_history: [1, 2] },
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

window.eval(appjs);
window.dispatchEvent(new window.Event("load"));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
let fails = 0;
const check = (name, cond, extra = "") => { console.log((cond ? "PASS" : "FAIL") + ": " + name + (extra ? "  [" + extra + "]" : "")); if (!cond) fails++; };
const setVals = (prop) => calls.filter((c) => c[0] === "set:" + prop).map((c) => c[1]);
const methodCalls = (m) => calls.filter((c) => c[0] === m);

await wait(300);

// ---------- 1. threshold lives INSIDE the details popover ----------
check("1a: NO standalone threshold button in .rps-head", !document.querySelector("#ov-rps-thresh"), "");

const dbtn = document.querySelector("#ov-rps-details");
dbtn.click();
await wait(20);
const pop = document.querySelector(".det-pop");
check("1b: details popover opens", !!pop, "");
const thrRow = pop && pop.querySelector(".dp-thr-row");
check("1c: threshold section present (title + value)", !!thrRow && thrRow.textContent.includes("name_id threshold") && thrRow.textContent.includes("≥ 33%"), thrRow && thrRow.textContent);
const slider = pop && pop.querySelector("input[type=range]");
check("1d: slider 0..100 step 1, value 33", !!slider && slider.min === "0" && slider.max === "100" && slider.step === "1" && slider.value === "33", slider && `${slider.min}..${slider.max}/${slider.step}=${slider.value}`);

// toggle gamma on (breakdown active), keep the popover open — the slider
// must live-update the chart without closing it
const drow = pop.querySelector(".dp-row input[type=checkbox]");
drow.checked = true;
drow.dispatchEvent(new window.Event("change", { bubbles: true }));
await wait(20);
calls = [];
await wait(2100); // next poll redraws with the stack (archive now has samples)

const swatch = document.querySelector("#ov-rps-legend .rl i").getAttribute("style").match(/#[0-9a-fA-F]{6}/)[0];
const gRgba = (a) => {
  const h = swatch.replace("#", "");
  return `rgba(${parseInt(h.slice(0, 2), 16)},${parseInt(h.slice(2, 4), 16)},${parseInt(h.slice(4, 6), 16)},${a})`;
};
const texts = () => methodCalls("fillText").map((c) => String(c[1]));

// default 33%: g1 (40%) kept alone, g2..g5 merged into "others (4)"
check("2a: g1 label drawn", texts().includes("g1"), JSON.stringify(texts()));
check("2b: merged band labeled 'others (4)'", texts().includes("others (4)"), JSON.stringify(texts()));
check("2c: g2..g5 labels NOT drawn (merged)", !texts().some((t) => ["g2", "g3", "g4", "g5"].includes(t)), JSON.stringify(texts()));
check("2d: hatch pattern requested for the others band", methodCalls("createPattern").length >= 1, String(methodCalls("createPattern").length));
check("2e: others band fills with translucent fallback (jsdom has no real pattern)", setVals("fillStyle").includes(gRgba(0.15)), "");
check("2f: single real band filled at 0.50 alpha", setVals("fillStyle").includes(gRgba(0.5)), "");

// ---------- 3. slider to 0% -> every band, graded 0.50 -> 0.10 ----------
const slider2 = document.querySelector(".det-pop input[type=range]");
check("3a-pre: popover still open (slider live)", !!slider2, "");
slider2.value = "0";
slider2.dispatchEvent(new window.Event("input", { bubbles: true }));
await wait(20);
check("3a: readout updates to ≥ 0%", document.querySelector(".det-pop .dp-thr-row").textContent.includes("≥ 0%"), "");
check("3b: threshold persisted", window.localStorage.getItem("xdb-rps-threshold") === "0", window.localStorage.getItem("xdb-rps-threshold"));
calls = [];
await wait(2100);
check("3c: all 5 name labels drawn at 0%", ["g1", "g2", "g3", "g4", "g5"].every((t) => texts().includes(t)), JSON.stringify(texts()));
check("3d: no 'others' band at 0%", !texts().some((t) => t.startsWith("others (")), JSON.stringify(texts()));
const alphas = setVals("fillStyle").filter((v) => v.startsWith("rgba(")).map((v) => parseFloat(v.match(/,\s*([\d.]+)\)$/)[1]));
check("3e: top band at 0.50", alphas.includes(0.5), JSON.stringify(alphas));
check("3f: faintest band at 0.10", alphas.includes(0.1), JSON.stringify(alphas));

// ---------- 4. slider to 60% -> only the top contributor ----------
const slider3 = document.querySelector(".det-pop input[type=range]");
slider3.value = "60";
slider3.dispatchEvent(new window.Event("input", { bubbles: true }));
await wait(20);
calls = [];
await wait(2100);
check("4a: at 60% only g1 + others (4) remain", texts().includes("g1") && texts().includes("others (4)") && !texts().includes("g2"), JSON.stringify(texts()));

// ---------- 5. hover lists the DRAWN bands (kept names + merged others) ----------
const canvas = document.querySelector("#ov-rps-canvas");
canvas.getBoundingClientRect = () => ({ left: 0, top: 0, width: 600, height: 190, right: 600, bottom: 190, x: 0, y: 0 });
calls = [];
canvas.dispatchEvent(new window.MouseEvent("mousemove", { clientX: 300, clientY: 60 }));
await wait(20);
const tt = texts();
// 2026-08-20: the tooltip mirrors the chart — merged name_ids appear as the
// single "others (N)" row, not as individual rows
check("5a: tooltip lists drawn bands (g1 + others, not merged names)", tt.includes("g1") && tt.includes("others (4)") && !["g2", "g3", "g4", "g5"].every((t) => tt.includes(t)), JSON.stringify(tt));

// ---------- 6. popover closes on outside mousedown, threshold survives ----------
document.body.dispatchEvent(new window.MouseEvent("mousedown", { bubbles: true }));
await wait(20);
check("6a: outside mousedown closes the popover", !document.querySelector(".det-pop"), "");
check("6b: threshold still persisted after close", window.localStorage.getItem("xdb-rps-threshold") === "60", window.localStorage.getItem("xdb-rps-threshold"));

console.log(fails ? `\n${fails} FAILURE(S)` : "\nALL PASS");
process.exit(fails ? 1 : 0);
