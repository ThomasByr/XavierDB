// Verification for the RPS chart Focus mode (2026-08-21):
//  1. Global ⇄ Focus segmented switch sits in the h3 between the title and
//     the summary; Global is the default; a ▾ arrow sits next to Focus
//  2. switching to Focus with NO saved app auto-opens the app picker under
//     the arrow (single-select, det-pop look); the arrow toggles it too
//  3. picking an app: persisted (xdb-rps-focus), title becomes "<app> · RPS",
//     legend lists the app's name_ids, summary counts name_id(s)
//  4. "Show details" is disabled in Focus, re-enabled in Global; clicking it
//     in Focus does nothing
//  5. reload with saved state (xdb-rps-mode=focus + app) → Focus active
//     directly, picker NOT auto-opened
//  6. Global mode unchanged: legend lists apps, summary counts app(s)
// Drives the repo bundle directly (same bytes as the compile-time embed).
import { JSDOM } from "jsdom";
import { readFileSync } from "fs";
import { fileURLToPath } from "node:url";

const REPO = process.env.XDB_REPO || fileURLToPath(new URL("../..", import.meta.url));
const html = readFileSync(REPO + "/src/assets/index.html", "utf8");
const appjs = readFileSync(REPO + "/src/assets/app.js", "utf8");

// app "alpha" (names a1,a2,a3) + app "beta" (name b1)
const mkMetrics = () => ({
  ts: Date.now(),
  qps: 30,
  config: { poll_seconds: 0.2, theme: "system", graph_smoothing: 5, cfg_version: 0, perms_version: 0, health_ttl_seconds: 5, multiplier: 1 },
  system: { cpu_pct: 5, mem_pct: 40, mem_used_mb: 100, mem_total_mb: 256, disk_pct: 30, disk_used_mb: 10, disk_total_mb: 50, net_rx_kbps: 1, net_tx_kbps: 1, uptime_s: 100, ts_ms: Date.now() },
  health: { status: "ok", mongodb: { ping_latency_ms: 1 }, app: { total_requests: 500 } },
  apps: [
    { app: "alpha", blocked: false, weight: 1, rps: 60, p50_ms: 3, limit: 100, rps_history: [1, 2, 3], names: [
      { name: "a1", id: "a1@alpha", blocked: false, rps: 30, p50_ms: 3, total_requests: 5, last_seen_ms: Date.now() - 30000, rps_history: [1, 2] },
      { name: "a2", id: "a2@alpha", blocked: false, rps: 20, p50_ms: 3, total_requests: 5, last_seen_ms: Date.now() - 30000, rps_history: [1, 2] },
      { name: "a3", id: "a3@alpha", blocked: false, rps: 10, p50_ms: 3, total_requests: 5, last_seen_ms: Date.now() - 30000, rps_history: [1, 2] },
    ] },
    { app: "beta", blocked: false, weight: 1, rps: 15, p50_ms: 3, limit: 100, rps_history: [1, 2, 3], names: [
      { name: "b1", id: "b1@beta", blocked: false, rps: 15, p50_ms: 3, total_requests: 5, last_seen_ms: Date.now() - 30000, rps_history: [1, 2] },
    ] },
  ],
  cursors: { count: 0, list: [] },
});

let fails = 0;
const check = (name, cond, extra = "") => { console.log((cond ? "PASS" : "FAIL") + ": " + name + (extra ? "  [" + extra + "]" : "")); if (!cond) fails++; };
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

async function boot(seed = {}) {
  const vc = new (await import("jsdom")).VirtualConsole();
  vc.on("jsdomError", (e) => console.log("[jsdomError]", e.message));
  const dom = new JSDOM(html, { url: "http://127.0.0.1:8000/dashboard/", runScripts: "outside-only", pretendToBeVisual: true, virtualConsole: vc });
  const { window } = dom;
  const { document } = window;
  window.matchMedia = () => ({ matches: false, addEventListener() {}, addListener() {} });
  window.structuredClone = (x) => JSON.parse(JSON.stringify(x));
  // recording 2d context (jsdom has no canvas)
  const ctxStub = new Proxy({}, {
    get: (t, k) => {
      if (k === "canvas") return {};
      if (k === "measureText") return (text) => ({ width: String(text).length * 5.5 });
      return (...args) => {};
    },
    set: () => true,
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
  for (const [k, v] of Object.entries(seed)) window.localStorage.setItem(k, v);
  window.eval(appjs);
  window.dispatchEvent(new window.Event("load"));
  await wait(700); // a few polls → archive has samples
  return { window, document };
}

const legendText = (document) => document.querySelector("#ov-rps-legend").textContent;

/* ---------- scenario A: fresh client ---------- */
{
  const { window, document } = await boot();
  const h3 = document.querySelector("#ov-rps h3");

  // 1. switch placement + defaults
  const mode = document.querySelector("#ov-rps-mode");
  check("1a: mode switch exists, inside the h3", !!mode && mode.parentElement === h3, "");
  const kids = [...h3.children].map((c) => c.id);
  check("1b: order title → mode → summary", kids[0] === "ov-rps-title" && kids[1] === "ov-rps-mode" && kids[2] === "ov-rps-summary", JSON.stringify(kids));
  check("1c: Global + Focus options + ▾ arrow", !!mode.querySelector("#ov-rps-mode-global") && !!mode.querySelector("#ov-rps-mode-focus") && mode.querySelector("#ov-rps-mode-arrow").textContent === "▾", "");
  check("1d: default mode is global", mode.getAttribute("data-mode") === "global" && document.querySelector("#ov-rps-mode-global").classList.contains("on"), "");
  check("1e: global legend lists apps", legendText(document).includes("alpha ·") && legendText(document).includes("beta ·"), legendText(document));
  check("1f: summary counts app(s)", document.querySelector("#ov-rps-summary").textContent.includes("2 app(s) · shared scale"), document.querySelector("#ov-rps-summary").textContent);
  const dbtn = document.querySelector("#ov-rps-details");
  check("1g: Show details enabled in Global", !dbtn.hasAttribute("disabled"), "");

  // 2. switch to Focus with no saved app → picker auto-opens
  document.querySelector("#ov-rps-mode-focus").click();
  await wait(20);
  let pop = document.querySelector(".focus-pop");
  check("2a: picker auto-opened under the arrow", !!pop && pop.parentElement === mode, "");
  const rows = pop ? [...pop.querySelectorAll(".fp-row .dp-name")].map((n) => n.textContent) : [];
  check("2b: picker lists both apps, none selected", rows.join(",") === "alpha,beta" && !pop.querySelector(".fp-row.sel"), JSON.stringify(rows));
  check("2c: mode persisted", window.localStorage.getItem("xdb-rps-mode") === "focus", "");
  check("2d: Show details disabled in Focus", dbtn.hasAttribute("disabled"), "");

  // arrow toggles the picker
  document.querySelector("#ov-rps-mode-arrow").click();
  await wait(20);
  check("2e: arrow click closes the picker", !document.querySelector(".focus-pop"), "");
  document.querySelector("#ov-rps-mode-arrow").click();
  await wait(20);
  check("2f: arrow click re-opens the picker", !!document.querySelector(".focus-pop"), "");

  // clicking Show details while disabled must do nothing
  dbtn.click();
  await wait(20);
  check("2g: Show details click is a no-op in Focus", !document.querySelector(".det-pop"), "");

  // 3. pick alpha
  pop = document.querySelector(".focus-pop");
  const alphaRow = [...pop.querySelectorAll(".fp-row")].find((r) => r.querySelector(".dp-name").textContent === "alpha");
  alphaRow.click();
  await wait(20);
  check("3a: picker closed after selection", !document.querySelector(".focus-pop"), "");
  check("3b: app persisted", window.localStorage.getItem("xdb-rps-focus") === "alpha", "");
  check("3c: title shows the focused app", document.querySelector("#ov-rps-title").textContent === "alpha · RPS", document.querySelector("#ov-rps-title").textContent);
  await wait(500); // next poll redraws with name series
  const lt = legendText(document);
  check("3d: legend lists name_ids of alpha", lt.includes("a1 ·") && lt.includes("a2 ·") && lt.includes("a3 ·"), lt);
  check("3e: beta's app line not in legend", !lt.includes("beta ·"), lt);
  const st = document.querySelector("#ov-rps-summary").textContent;
  check("3f: summary counts name_id(s)", st.includes("3 name_id(s) · shared scale"), st);
  check("3g: legend swatch count = 3 names", document.querySelectorAll("#ov-rps-legend .rl").length === 3, "");

  // 4. back to Global
  document.querySelector("#ov-rps-mode-global").click();
  await wait(20);
  check("4a: mode back to global", document.querySelector("#ov-rps-mode").getAttribute("data-mode") === "global" && !dbtn.hasAttribute("disabled"), "");
  await wait(500);
  check("4b: legend back to apps", legendText(document).includes("alpha ·") && legendText(document).includes("beta ·"), legendText(document));
  check("4c: title back to All apps", document.querySelector("#ov-rps-title").textContent === "All apps · RPS", "");
  window.close();
}

/* ---------- scenario B: saved focus state ---------- */
{
  const { window, document } = await boot({ "xdb-rps-mode": "focus", "xdb-rps-focus": "beta" });
  const mode = document.querySelector("#ov-rps-mode");
  check("5a: focus mode restored from localStorage", mode.getAttribute("data-mode") === "focus" && document.querySelector("#ov-rps-mode-focus").classList.contains("on"), "");
  check("5b: picker NOT auto-opened (app saved)", !document.querySelector(".focus-pop"), "");
  check("5c: title = beta · RPS", document.querySelector("#ov-rps-title").textContent === "beta · RPS", "");
  const lt = legendText(document);
  check("5d: legend lists beta's name_ids only", lt.includes("b1 ·") && !lt.includes("a1 ·") && !lt.includes("alpha ·"), lt);
  const st = document.querySelector("#ov-rps-summary").textContent;
  check("5e: summary counts name_id(s)", st.includes("1 name_id(s) · shared scale"), st);
  check("5f: Show details disabled", document.querySelector("#ov-rps-details").hasAttribute("disabled"), "");
  window.close();
}

console.log(fails ? `\n${fails} FAILURE(S)` : "\nALL PASS");
process.exit(fails ? 1 : 0);
