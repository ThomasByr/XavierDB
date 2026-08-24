import { JSDOM } from "jsdom";
import { readFileSync } from "fs";
import { fileURLToPath } from "node:url";

// Bug scenario: dashboard.log_level=debug flooded the newest window with
// DEBUG lines; the INFO lines the user filters for are older than the first
// 300 entries. The logs tab must auto-page older history instead of showing
// nothing. Also checks a composed filter that matches nothing anywhere.
// Drives the repo bundle directly (same bytes as the compile-time embed).
const REPO = process.env.XDB_REPO || fileURLToPath(new URL("../..", import.meta.url));
const html = readFileSync(REPO + "/src/assets/index.html", "utf8");
const appjs = readFileSync(REPO + "/src/assets/app.js", "utf8");

// history: seqs 0..699 (700 lines). Oldest 50: INFO. Rest: DEBUG (flood).
const mkLine = (seq, level) => ({
  seq,
  raw: `2026-08-14T15:${String(Math.floor(seq / 60)).padStart(2, "0")}:${String(seq % 60).padStart(2, "0")}Z ${level.padStart(5)} XavierDB: line ${seq}`,
  level,
  logger: "XavierDB",
  app: level === "INFO" ? "appx" : null,
  name: level === "INFO" ? "t" : null,
});
const HIST = [];
for (let s = 0; s < 50; s++) HIST.push(mkLine(s, "INFO"));
for (let s = 50; s < 700; s++) HIST.push(mkLine(s, "DEBUG"));
let logCalls = [];
const page = (before) => {
  const eligible = before === undefined ? HIST : HIST.filter((l) => l.seq < before);
  return {
    lines: eligible.slice(-300),
    total: HIST.length,
    apps: ["appx"],
    names: [{ app: "appx", name: "t" }],
    loggers: ["XavierDB"],
    retention: { files: 3, size_mb: 1, path: "xavierdb.log" },
  };
};

const vc = new (await import("jsdom")).VirtualConsole();
vc.on("jsdomError", (e) => console.log("[jsdomError]", e.message, e.detail || ""));
const dom = new JSDOM(html, { url: "http://127.0.0.1:8000/dashboard/", runScripts: "outside-only", pretendToBeVisual: true, virtualConsole: vc });
const { window } = dom;
const { document } = window;
window.matchMedia = () => ({ matches: false, addEventListener() {}, addListener() {} });
if (!window.structuredClone) window.structuredClone = (x) => JSON.parse(JSON.stringify(x));
window.URL.createObjectURL = () => "blob:stub";
window.fetch = async (url) => {
  const u = new URL(String(url), "http://127.0.0.1:8000");
  const body = u.pathname.includes("/dashboard/api/logs")
    ? page(u.searchParams.get("before") ? Number(u.searchParams.get("before")) : undefined)
    : u.pathname.includes("/dashboard/api/session") ? { username: "admin" }
    : u.pathname.includes("/dashboard/api/metrics") ? { ts: 0, qps: 0, config: { poll_seconds: 2, theme: "system", graph_smoothing: 5, cfg_version: 0, perms_version: 0, health_ttl_seconds: 5, multiplier: 1 }, system: {}, health: {}, apps: [], cursors: { count: 0, list: [] } }
    : { ok: true };
  if (u.pathname.includes("/dashboard/api/logs"))
    logCalls.push(u.searchParams.get("before") || "initial");
  return { ok: true, status: 200, json: async () => body };
};

window.eval(appjs);
window.dispatchEvent(new window.Event("load"));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

window.location.hash = "#/logs";
window.dispatchEvent(new window.HashChangeEvent("hashchange"));
await wait(200);
const box = () => document.querySelector("#logs-box");
const status = () => document.querySelector("#logs-status")?.textContent || "";
const mousedown = (el) => el.dispatchEvent(new window.MouseEvent("mousedown", { bubbles: true, cancelable: true }));
const click = (el) => el.dispatchEvent(new window.MouseEvent("click", { bubbles: true, cancelable: true }));
let fails = 0;
const check = (name, ok) => { console.log((ok ? "PASS" : "FAIL") + ": " + name); if (!ok) fails++; };

check("initial load: 300 DEBUG rows", box().childElementCount === 300);
check("initial status cleared", status() === "");

// --- scenario A: the reported bug — filter INFO, newest window is all DEBUG
logCalls = [];
const pop = document.querySelector("#logs-pop");
mousedown(document.querySelector("#logs-fbtn")); click(document.querySelector("#logs-fbtn"));
const infoChip = Array.from(document.querySelectorAll("#fl-sugg .sugg")).find((c) => c.textContent.includes("INFO"));
mousedown(infoChip); click(infoChip);
await wait(600); // let the auto-paging loop run (3 pages: 700-300-300...)
const rowsA = box().childElementCount;
check("INFO filter: rows appeared (got " + rowsA + ")", rowsA === 50);
check("INFO filter: all rows are INFO", Array.from(box().children).every((r) => r.textContent.includes("INFO")));
check("INFO filter: auto-paged via before= (" + logCalls.join(",") + ")", logCalls.length >= 2 && logCalls.slice(1).every((b) => b !== "initial"));
check("status hidden after matches found", status() === "");

// --- scenario B: composed filters — INFO AND app appx (both must hold)
mousedown(document.querySelector("#logs-fbtn")); click(document.querySelector("#logs-fbtn"));
const cat = document.querySelector("#fl-cat");
cat.value = "app";
cat.dispatchEvent(new window.Event("change", { bubbles: true }));
await wait(50);
const appChip = Array.from(document.querySelectorAll("#fl-sugg .sugg")).find((c) => c.textContent.includes("appx"));
mousedown(appChip); click(appChip);
await wait(300);
check("composed INFO+app: still 50 rows", box().childElementCount === 50);

// --- scenario C: composed filters matching NOTHING anywhere -> exhaustion
// add name "zzz" (typed value): INFO rows carry name "t" -> AND excludes all
mousedown(document.querySelector("#logs-fbtn")); click(document.querySelector("#logs-fbtn"));
const cat3 = document.querySelector("#fl-cat");
cat3.value = "name";
cat3.dispatchEvent(new window.Event("change", { bubbles: true }));
await wait(30);
const valIn = document.querySelector("#fl-val");
valIn.value = "zzz";
valIn.dispatchEvent(new window.KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
await wait(800);
check("unmatched composed filter: 0 rows", box().childElementCount === 0);
check("unmatched composed filter: status says no match (" + status() + ")", status().includes("no lines match"));

console.log(fails === 0 ? "ALL PASS" : fails + " FAILURES");
process.exit(fails === 0 ? 0 : 1);
