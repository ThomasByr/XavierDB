// Config-tab dirty-save-indicator verification (2026-08-14, after user-approved UX change).
// Also re-runs the revert-on-reload scenarios: A (unsaved edit + reload) and B (save round-trip).
// Drives the repo bundle directly (same bytes as the compile-time embed).
import { JSDOM } from "jsdom";
import { readFileSync } from "fs";
import { fileURLToPath } from "node:url";

// Drives the repo bundle directly (same bytes as the compile-time embed).
const REPO = process.env.XDB_REPO || fileURLToPath(new URL("../..", import.meta.url));
const html = readFileSync(REPO + "/src/assets/index.html", "utf8");
const appjs = readFileSync(REPO + "/src/assets/app.js", "utf8");

// ---- server state simulation (like the real /dashboard/api/config) ----
let serverVersion = 41;
let serverConfig = {
  global: { jwt_token_lifetime_minutes: 90, permission_file: "authorized_keys.yml" },
  rate_limit: { multiplier: 1, target_latency_ms: 50, latency_sensitivity: 1, pressure_sensitivity: 1.5, growth_rate: 1.15, min_limit: 1, max_limit: 200, tick_seconds: 5, ema_alpha: 0.2 },
  dashboard: { poll_seconds: 2, graph_smoothing: 5, log_level: "info", theme: "system" },
  health: { cache_ttl_seconds: 5 },
  auth: { max_per_minute_per_ip: 30, session_ttl_hours: 24 },
  blocked: [],
};
let serverHistory = [{ ts: 1755000000, desc: "config edited from dashboard", path: "rate_limit.max_limit" }];
let postBodies = [];
const echo = () => ({ version: serverVersion, config: serverConfig, history: serverHistory, undo_available: true, redo_available: false });

const vc = new (await import("jsdom")).VirtualConsole();
vc.on("jsdomError", (e) => console.log("[jsdomError]", e.message, e.detail || ""));
const dom = new JSDOM(html, { url: "http://127.0.0.1:8000/dashboard/", runScripts: "outside-only", pretendToBeVisual: true, virtualConsole: vc });
const { window } = dom;
const { document } = window;

window.matchMedia = () => ({ matches: false, addEventListener() {}, addListener() {} });
if (!window.structuredClone) window.structuredClone = (x) => JSON.parse(JSON.stringify(x));
window.URL.createObjectURL = () => "blob:stub";
window.confirm = () => true;

window.fetch = async (url, opts = {}) => {
  const u = String(url);
  const method = (opts.method || "GET").toUpperCase();
  if (u.includes("/dashboard/api/config") && method === "POST") {
    const body = JSON.parse(opts.body);
    postBodies.push(body);
    serverConfig = body.config;                 // server accepts (no sanitize in jsdom)
    serverVersion += 1;
    serverHistory = [{ ts: 1755000001, desc: "config edited from dashboard", path: "rate_limit.max_limit" }, ...serverHistory];
    return { ok: true, status: 200, json: async () => echo() };
  }
  if (u.includes("/dashboard/api/config")) return { ok: true, status: 200, json: async () => echo() };
  if (u.includes("/dashboard/api/session")) return { ok: true, status: 200, json: async () => ({ username: "admin" }) };
  if (u.includes("/dashboard/api/metrics")) return { ok: true, status: 200, json: async () => ({ ts: 0, qps: 0, config: { poll_seconds: 2, theme: "system", graph_smoothing: 5, cfg_version: 0, perms_version: 0, health_ttl_seconds: 5, multiplier: 1 }, system: {}, health: {}, apps: [], cursors: { count: 0, list: [] } }) };
  return { ok: true, status: 200, json: async () => ({ ok: true }) };
};

window.eval(appjs);
window.dispatchEvent(new window.Event("load"));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

const enterConfig = async () => {
  window.location.hash = "#/config";
  window.dispatchEvent(new window.HashChangeEvent("hashchange"));
  await wait(150); // loadConfig fetch + renderConfigForm
};
const slider = (path) => document.querySelector(`input[data-path="${path}"]`);
const setSlider = (el, v) => {
  el.value = String(v);
  el.dispatchEvent(new window.Event("input", { bubbles: true }));
  el.dispatchEvent(new window.Event("change", { bubbles: true }));
};
const dirtyBadge = () => document.querySelector("#cfg-dirty");
const saveBtn = () => document.querySelector("#cfg-save");

let fails = 0;
const check = (name, cond, extra = "") => { console.log((cond ? "PASS" : "FAIL") + ": " + name + (extra ? "  [" + extra + "]" : "")); if (!cond) fails++; };

// ---------- Scenario A: fresh render -> clean state ----------
await enterConfig();
const maxSl = slider("rate_limit.max_limit");
check("A1: config form rendered, max_limit slider at server value 200", !!maxSl && maxSl.value === "200", "value=" + (maxSl && maxSl.value));
check("A2: dirty badge hidden on load", dirtyBadge().classList.contains("hidden"));
const badgeParent = dirtyBadge().parentElement;
check("A2b: badge lives in the title h3, right-anchored (margin-left:auto), NOT in the buttons row", badgeParent.tagName === "H3" && dirtyBadge().style.marginLeft === "auto" && !badgeParent.closest(".row"), "parent=" + badgeParent.tagName + " marginLeft=" + dirtyBadge().style.marginLeft);
check("A3: Save disabled when clean", saveBtn().disabled === true, "disabled=" + saveBtn().disabled);

// ---------- Scenario B: edit marks dirty + enables Save ----------
setSlider(maxSl, 900);
check("B1: slider reads 900, readout live-updates", maxSl.value === "900" && maxSl.closest(".sf").querySelector(".sf-value").textContent === "900");
check("B2: dirty badge shown after edit", !dirtyBadge().classList.contains("hidden"));
check("B3: Save enabled when dirty", saveBtn().disabled === false, "disabled=" + saveBtn().disabled);

// select + text fields also mark dirty
// (2026-08-24: was dashboard.theme — that select moved to the topbar in the
// 2026-08-21 config-tab restructure; log_level is the remaining select)
const themeSel = document.querySelector('select[data-path="dashboard.log_level"]');
themeSel.value = "debug";
themeSel.dispatchEvent(new window.Event("change", { bubbles: true }));
check("B4: select change keeps dirty", !dirtyBadge().classList.contains("hidden"));

// ---------- Scenario C: reload discards unsaved edits (original user report) ----------
await enterConfig();
const maxSl2 = slider("rate_limit.max_limit");
check("C1: after reload without save, slider back to 200", maxSl2.value === "200", "value=" + maxSl2.value);
check("C2: dirty badge hidden again", dirtyBadge().classList.contains("hidden"));
check("C3: Save disabled again", saveBtn().disabled === true);

// ---------- Scenario D: save round-trip with the indicator ----------
setSlider(slider("rate_limit.max_limit"), 700);
setSlider(slider("rate_limit.min_limit"), 80);
await wait(10);
saveBtn().dispatchEvent(new window.MouseEvent("click", { bubbles: true, cancelable: true }));
await wait(200); // POST + renderConfigForm
check("D1: POST /config captured with new values", postBodies.length === 1 && postBodies[0].config.rate_limit.max_limit === 700 && postBodies[0].config.rate_limit.min_limit === 80,
  JSON.stringify(postBodies[0] && postBodies[0].config.rate_limit));
const maxSl3 = slider("rate_limit.max_limit");
check("D2: after save, re-render shows 700", maxSl3.value === "700", "value=" + maxSl3.value);
check("D3: dirty badge hidden after save", dirtyBadge().classList.contains("hidden"));
check("D4: Save disabled again after save", saveBtn().disabled === true);

// ---------- Scenario E: saved values persist across reload ----------
await enterConfig();
check("E1: reload shows persisted values", slider("rate_limit.max_limit").value === "700" && slider("rate_limit.min_limit").value === "80");
check("E2: badge hidden, save disabled", dirtyBadge().classList.contains("hidden") && saveBtn().disabled === true);

console.log(fails === 0 ? "\nALL PASS" : `\n${fails} FAILURE(S)`);
process.exit(fails === 0 ? 0 : 1);
