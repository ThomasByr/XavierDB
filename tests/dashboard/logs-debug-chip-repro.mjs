import { JSDOM } from "jsdom";
import { readFileSync } from "fs";
import { fileURLToPath } from "node:url";

// Drives the repo bundle directly (same bytes as the compile-time embed).
const REPO = process.env.XDB_REPO || fileURLToPath(new URL("../..", import.meta.url));
const html = readFileSync(REPO + "/src/assets/index.html", "utf8");
const appjs = readFileSync(REPO + "/src/assets/app.js", "utf8");

const vc = new (await import("jsdom")).VirtualConsole();
vc.on("jsdomError", e => console.log("[jsdomError]", e.message, e.detail || ""));
vc.on("error", (...a) => console.log("[error]", ...a));
const dom = new JSDOM(html, { url: "http://127.0.0.1:8000/dashboard/", runScripts: "outside-only", pretendToBeVisual: true, virtualConsole: vc });
const { window } = dom;
const { document } = window;

window.matchMedia = () => ({ matches: false, addEventListener() {}, addListener() {} });
if (!window.structuredClone) window.structuredClone = (x) => JSON.parse(JSON.stringify(x));
window.URL.createObjectURL = () => "blob:stub";
const LOGS = {
  lines: [
    { seq: 0, raw: "2026-08-14T15:00:00.000000Z  INFO XavierDB: listening", level: "INFO", logger: "XavierDB", app: null, name: null },
    { seq: 1, raw: "2026-08-14T15:00:01.000000Z DEBUG XavierDB::routes_q: GET /q/a/b as t@x", level: "DEBUG", logger: "XavierDB::routes_q", app: "x", name: "t" },
    { seq: 2, raw: "2026-08-14T15:00:02.000000Z  WARN XavierDB: warn line", level: "WARN", logger: "XavierDB", app: null, name: null },
  ],
  total: 3, apps: ["x"], names: [{ app: "x", name: "t" }], loggers: ["XavierDB", "XavierDB::routes_q"],
};
window.fetch = async (url) => {
  const u = String(url);
  const body = u.includes("/dashboard/api/logs") ? LOGS
    : u.includes("/dashboard/api/session") ? { username: "admin" }
    : u.includes("/dashboard/api/metrics") ? { ts: 0, qps: 0, config: { poll_seconds: 2, theme: "system", graph_smoothing: 5, cfg_version: 0, perms_version: 0, health_ttl_seconds: 5, multiplier: 1 }, system: {}, health: {}, apps: [], cursors: { count: 0, list: [] } }
    : { ok: true };
  return { ok: true, status: 200, json: async () => body };
};

window.eval(appjs);
window.dispatchEvent(new window.Event("load"));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

window.location.hash = "#/logs";
window.dispatchEvent(new window.HashChangeEvent("hashchange"));
await wait(150);
console.log("view has logs-filterbar:", !!document.querySelector(".logs-filterbar"), "| app hidden:", document.querySelector("#app")?.classList.contains("hidden"));

const pop = document.querySelector("#logs-pop");
const fbtn = document.querySelector("#logs-fbtn");
const badgeBox = document.querySelector("#logs-fbadges");
const mousedown = (el) => el.dispatchEvent(new window.MouseEvent("mousedown", { bubbles: true, cancelable: true }));
const click = (el) => el.dispatchEvent(new window.MouseEvent("click", { bubbles: true, cancelable: true }));

mousedown(fbtn); click(fbtn);
console.log("after open: popover open =", pop.classList.contains("open"));
if (!pop.classList.contains("open")) { console.log("FAIL: popover did not open"); process.exit(1); }

const chips = Array.from(document.querySelectorAll("#fl-sugg .sugg"));
const dbg = chips.find((c) => c.textContent.includes("DEBUG"));
console.log("chips:", chips.map((c) => c.textContent).join(", "));
if (!dbg) { console.log("FAIL: DEBUG chip not found"); process.exit(1); }
console.log("DEBUG chip inside filterbar:", document.querySelector(".logs-filterbar").contains(dbg));

mousedown(dbg); click(dbg);
await wait(30);
console.log("after chip click: popover open =", pop.classList.contains("open"), "| badges =", JSON.stringify(badgeBox.textContent));
const added = badgeBox.textContent.includes("DEBUG");
console.log(added && pop.classList.contains("open") ? "PASS: filter added, popover stays open" : "FAIL: " + (pop.classList.contains("open") ? "filter not added" : "popover closed"));
// NOTE: polling now runs on every tab (RPS archive sampling) — exit explicitly
process.exit(added && pop.classList.contains("open") ? 0 : 1);
