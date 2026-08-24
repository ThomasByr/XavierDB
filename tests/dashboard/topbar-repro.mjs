// Topbar repro (2026-08-21): theme button (sun/moon + label), settings popover
// (graph smoothing + poll interval sliders, localStorage persistence).
// Drives the REAL served bundle fetched fresh from the running server.
import { JSDOM } from "jsdom";
import { readFileSync } from "fs";
import { fileURLToPath } from "node:url";

// Drives the repo bundle directly (same bytes as the compile-time embed).
const REPO = process.env.XDB_REPO || fileURLToPath(new URL("../..", import.meta.url));
const html = readFileSync(REPO + "/src/assets/index.html", "utf8");
const appjs = readFileSync(REPO + "/src/assets/app.js", "utf8");

const vc = new (await import("jsdom")).VirtualConsole();
vc.on("jsdomError", (e) => console.log("[jsdomError]", e.message, e.detail || ""));
const dom = new JSDOM(html, { url: "http://127.0.0.1:8000/dashboard/", runScripts: "outside-only", pretendToBeVisual: true, virtualConsole: vc });
const { window } = dom;
const { document } = window;

window.matchMedia = () => ({ matches: true, addEventListener() {}, addListener() {} });
if (!window.structuredClone) window.structuredClone = (x) => JSON.parse(JSON.stringify(x));
window.HTMLCanvasElement.prototype.getContext = () => new Proxy({}, { get: () => () => ({ width: 10 }) });

window.fetch = async (url, opts = {}) => {
  const u = String(url);
  if (u.includes("/dashboard/api/session")) return { ok: true, status: 200, json: async () => ({ username: "admin" }) };
  if (u.includes("/dashboard/api/metrics"))
    return {
      ok: true,
      status: 200,
      json: async () => ({
        ts: 0,
        qps: 0,
        config: { cfg_version: 0, perms_version: 0, health_ttl_seconds: 5, multiplier: 1 },
        system: {},
        health: {},
        apps: [],
        cursors: { count: 0, list: [] },
      }),
    };
  return { ok: true, status: 200, json: async () => ({ ok: true }) };
};

window.eval(appjs);
window.dispatchEvent(new window.Event("load"));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
await wait(200);

let fails = 0;
const check = (name, cond, extra = "") => {
  console.log((cond ? "PASS" : "FAIL") + ": " + name + (extra ? "  [" + extra + "]" : ""));
  if (!cond) fails++;
};

// ---------- theme: default dark ----------
const themeBtn = document.getElementById("theme-btn");
check("default theme is dark", document.documentElement.dataset.theme === "dark");
check("theme label says Dark", document.getElementById("theme-label").textContent === "Dark");
check("moon icon shown in dark", themeBtn.hasAttribute("data-dark"));

// ---------- theme: toggle to light ----------
themeBtn.click();
check("click → light theme", document.documentElement.dataset.theme === "light");
check("label says Light", document.getElementById("theme-label").textContent === "Light");
check("sun icon shown in light", !themeBtn.hasAttribute("data-dark"));
check("localStorage xdb-theme=light", window.localStorage.getItem("xdb-theme") === "light");

// toggle back to dark
themeBtn.click();
check("click again → dark", document.documentElement.dataset.theme === "dark");

// ---------- settings popover ----------
const settingsBtn = document.getElementById("settings-btn");
settingsBtn.click();
await wait(50);
let pop = document.querySelector(".settings-pop");
check("popover opens", !!pop);
const sliders = pop ? pop.querySelectorAll("input[type=range]") : [];
check("two sliders", sliders.length === 2, String(sliders.length));
const vals = pop ? Array.from(pop.querySelectorAll(".sp-val")).map((n) => n.textContent) : [];
check("smoothing default 5 samples", vals[0] === "5 samples", vals.join(", "));
check("poll default 2.0 s", vals[1] === "2.0 s", vals.join(", "));

// move both sliders (input then change)
if (pop) {
  const [sm, pl] = sliders;
  sm.value = "12";
  sm.dispatchEvent(new window.Event("input", { bubbles: true }));
  sm.dispatchEvent(new window.Event("change", { bubbles: true }));
  pl.value = "0.5";
  pl.dispatchEvent(new window.Event("input", { bubbles: true }));
  pl.dispatchEvent(new window.Event("change", { bubbles: true }));
  check("live value updates (smoothing)", pop.querySelectorAll(".sp-val")[0].textContent === "12 samples");
  check("live value updates (poll)", pop.querySelectorAll(".sp-val")[1].textContent === "0.5 s");
}
check("localStorage xdb-smoothing=12", window.localStorage.getItem("xdb-smoothing") === "12");
check("localStorage xdb-poll=0.5", window.localStorage.getItem("xdb-poll") === "0.5");

// ---------- outside click closes ----------
document.body.dispatchEvent(new window.MouseEvent("mousedown", { bubbles: true }));
await wait(50);
check("outside mousedown closes popover", !document.querySelector(".settings-pop"));

// ---------- persistence across reload ----------
const dom2 = new JSDOM(html, { url: "http://127.0.0.1:8000/dashboard/", runScripts: "outside-only", pretendToBeVisual: true, virtualConsole: vc });
const w2 = dom2.window;
w2.matchMedia = window.matchMedia;
w2.HTMLCanvasElement.prototype.getContext = () => new Proxy({}, { get: () => () => ({ width: 10 }) });
w2.fetch = window.fetch;
w2.localStorage.setItem("xdb-theme", "light");
w2.localStorage.setItem("xdb-smoothing", "12");
w2.localStorage.setItem("xdb-poll", "0.5");
w2.eval(appjs);
w2.dispatchEvent(new w2.Event("load"));
await wait(200);
check("reload: light restored", w2.document.documentElement.dataset.theme === "light");
check("reload: label Light", w2.document.getElementById("theme-label").textContent === "Light");

console.log(fails === 0 ? "\nALL PASS" : `\n${fails} FAILURES`);
process.exit(fails === 0 ? 0 : 1);
