// Config-tab section layout repro (2026-08-21): exactly two sections
// (General + Rate limiting) side by side; Health TTL + Log level live in General.
import { JSDOM } from "jsdom";
import { readFileSync } from "fs";
import { fileURLToPath } from "node:url";

// Drives the repo bundle directly (same bytes as the compile-time embed).
const REPO = process.env.XDB_REPO || fileURLToPath(new URL("../..", import.meta.url));
const html = readFileSync(REPO + "/src/assets/index.html", "utf8");
const appjs = readFileSync(REPO + "/src/assets/app.js", "utf8");

let serverConfig = {
  global: { jwt_token_lifetime_minutes: 90, permission_file: "authorized_keys.yml" },
  rate_limit: { multiplier: 1, target_latency_ms: 50, latency_sensitivity: 1, pressure_sensitivity: 1.5, growth_rate: 1.15, min_limit: 1, max_limit: 200, tick_seconds: 5, ema_alpha: 0.2 },
  dashboard: { log_level: "info" },
  health: { cache_ttl_seconds: 5 },
  auth: { max_per_minute_per_ip: 30, session_ttl_hours: 24 },
  blocked: [],
};
const echo = () => ({ version: 1, config: serverConfig, history: [], undo_available: false, redo_available: false });

const vc = new (await import("jsdom")).VirtualConsole();
vc.on("jsdomError", (e) => console.log("[jsdomError]", e.message, e.detail || ""));
const dom = new JSDOM(html, { url: "http://127.0.0.1:8000/dashboard/", runScripts: "outside-only", pretendToBeVisual: true, virtualConsole: vc });
const { window } = dom;
const { document } = window;

window.matchMedia = () => ({ matches: false, addEventListener() {}, addListener() {} });
if (!window.structuredClone) window.structuredClone = (x) => JSON.parse(JSON.stringify(x));
window.fetch = async (url, opts = {}) => {
  const u = String(url);
  if (u.includes("/dashboard/api/config") && (opts.method || "GET").toUpperCase() === "POST") {
    serverConfig = JSON.parse(opts.body).config;
    return { ok: true, status: 200, json: async () => echo() };
  }
  if (u.includes("/dashboard/api/config")) return { ok: true, status: 200, json: async () => echo() };
  if (u.includes("/dashboard/api/session")) return { ok: true, status: 200, json: async () => ({ username: "admin" }) };
  if (u.includes("/dashboard/api/metrics"))
    return {
      ok: true,
      status: 200,
      json: async () => ({ ts: 0, qps: 0, config: { cfg_version: 0, perms_version: 0, health_ttl_seconds: 5, multiplier: 1 }, system: {}, health: {}, apps: [], cursors: { count: 0, list: [] } }),
    };
  return { ok: true, status: 200, json: async () => ({ ok: true }) };
};

window.eval(appjs);
window.dispatchEvent(new window.Event("load"));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
window.location.hash = "#/config";
window.dispatchEvent(new window.HashChangeEvent("hashchange"));
await wait(150);

let fails = 0;
const check = (name, cond, extra = "") => {
  console.log((cond ? "PASS" : "FAIL") + ": " + name + (extra ? "  [" + extra + "]" : ""));
  if (!cond) fails++;
};

const cards = Array.from(document.querySelectorAll("#cfg-form .card h3")).map((n) => n.textContent);
check("exactly two sections", cards.length === 2, cards.join(" + "));
check("section 1 is General", cards[0] === "General");
check("section 2 is Rate limiting", cards[1] === "Rate limiting");
check("no Dashboard section", !cards.includes("Dashboard"));

const paths = Array.from(document.querySelectorAll("#cfg-form [data-path]")).map((n) => n.dataset.path);
check("health TTL in form", paths.includes("health.cache_ttl_seconds"));
check("log level in form", paths.includes("dashboard.log_level"));
const generalCard = Array.from(document.querySelectorAll("#cfg-form .card")).find((c) => c.querySelector("h3")?.textContent === "General");
const generalPaths = Array.from(generalCard.querySelectorAll("[data-path]")).map((n) => n.dataset.path);
check("health TTL in General", generalPaths.includes("health.cache_ttl_seconds"));
check("log level in General", generalPaths.includes("dashboard.log_level"));
check("log level is a select", generalCard.querySelector('select[data-path="dashboard.log_level"]') !== null);

// log level select actually saves
const sel = generalCard.querySelector('select[data-path="dashboard.log_level"]');
sel.value = "debug";
sel.dispatchEvent(new window.Event("change", { bubbles: true }));
await wait(50);
document.querySelector("#cfg-save").click();
await wait(150);
check("log level=debug saved to server", serverConfig.dashboard.log_level === "debug", serverConfig.dashboard.log_level);

console.log(fails === 0 ? "\nALL PASS" : `\n${fails} FAILURES`);
process.exit(fails === 0 ? 0 : 1);
