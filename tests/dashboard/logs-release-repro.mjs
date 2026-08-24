// Logs tab memory release: verifies two additions to view-logs.ts —
//   A) "Clear all" resets to the default unfiltered view (fresh newest page, 300 rows),
//   B) leaving + returning to the Logs tab keeps the active filters, re-finds the
//      matching rows, and doesn't let a stale in-flight page repopulate the cleared ring.
// Drives the SERVED bundle (re-fetch after every server rebuild — compile-time embed).
import { JSDOM } from "jsdom";
import { readFileSync } from "fs";
import { fileURLToPath } from "node:url";

// Drives the repo bundle directly (same bytes as the compile-time embed).
const REPO = process.env.XDB_REPO || fileURLToPath(new URL("../..", import.meta.url));
const html = readFileSync(REPO + "/src/assets/index.html", "utf8");
const appjs = readFileSync(REPO + "/src/assets/app.js", "utf8");

// history: 800 lines, oldest->newest. Newest 300 (seq 600..799) are DEBUG; the
// 150 INFO lines are older (seq 0..149). So an INFO filter forces older-page paging.
const mkLine = (seq, level) => ({
  seq,
  raw: `2026-08-14T15:00Z ${level.padStart(5)} XavierDB: line ${seq}`,
  level,
  logger: "XavierDB",
  app: level === "INFO" ? "appx" : null,
  name: level === "INFO" ? "t" : null,
});
const N = 800, INFO_COUNT = 150;
const HIST = [];
for (let s = 0; s < INFO_COUNT; s++) HIST.push(mkLine(s, "INFO"));
for (let s = INFO_COUNT; s < N; s++) HIST.push(mkLine(s, "DEBUG"));

let logCalls = [];
let pageDelay = 0; // ms to stall paged (before=) requests, to expose the race
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
  let body;
  if (u.pathname.includes("/dashboard/api/logs")) {
    const before = u.searchParams.get("before");
    logCalls.push(before || "initial");
    if (before && pageDelay) await new Promise((r) => setTimeout(r, pageDelay));
    body = page(before ? Number(before) : undefined);
  } else if (u.pathname.includes("/dashboard/api/session")) body = { username: "admin" };
  else if (u.pathname.includes("/dashboard/api/metrics"))
    body = { ts: 0, qps: 0, config: { poll_seconds: 2 }, system: {}, health: {}, apps: [], cursors: { count: 0, list: [] } };
  else body = { ok: true };
  return { ok: true, status: 200, json: async () => body };
};

window.eval(appjs);
window.dispatchEvent(new window.Event("load"));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const mousedown = (el) => el && el.dispatchEvent(new window.MouseEvent("mousedown", { bubbles: true, cancelable: true }));
const click = (el) => el && el.dispatchEvent(new window.MouseEvent("click", { bubbles: true, cancelable: true }));
const box = () => document.querySelector("#logs-box");
const go = async (route, ms) => { window.location.hash = "#/" + route; window.dispatchEvent(new window.HashChangeEvent("hashchange")); await wait(ms); };
const addFilter = async (value) => {
  mousedown(document.querySelector("#logs-fbtn")); click(document.querySelector("#logs-fbtn"));
  const chip = Array.from(document.querySelectorAll("#fl-sugg .sugg")).find((c) => c.textContent.includes(value));
  mousedown(chip); click(chip);
};
let fails = 0;
const check = (name, ok) => { console.log((ok ? "PASS" : "FAIL") + ": " + name); if (!ok) fails++; };

// --- boot into logs
await go("logs", 400);
check("initial: newest 300 DEBUG rows", box().childElementCount === 300);
check("initial: first /logs call was latest page (no before)", logCalls[0] === "initial");

// --- A) clear-all resets to a default unfiltered latest page, even with a paged fetch in flight
pageDelay = 200;
logCalls = [];
await addFilter("INFO");          // newest page is all DEBUG, so this starts older-page paging (slow)
await wait(30);                  // let a paged request go in flight
click(document.querySelector("#fl-clear")); // clear-all while paging is pending
await wait(400);                  // let the stale page resolve and be discarded
const rowsA = box().childElementCount;
check("clear-all: back to a fresh unfiltered page (last /logs call = latest page)", logCalls.length > 0 && logCalls[logCalls.length - 1] === "initial");
check("clear-all: box shows the newest 300 (got " + rowsA + ")", rowsA === 300);
check("clear-all: no stale INFO page spliced in", rowsA === 300 && Array.from(box().children).every((r) => r.textContent.includes("DEBUG") && !r.textContent.includes("INFO")));

// --- B) filters persist across leave+return, and matches are re-found after release
pageDelay = 0;
logCalls = [];
await addFilter("INFO"); // re-apply the filter (cleared above)
await wait(700);   // auto-page until INFO is found
const rowsB = box().childElementCount;
check("INFO re-found after re-apply (got " + rowsB + ")", rowsB > 0 && Array.from(box().children).every((r) => r.textContent.includes("INFO")));

// leave to a canvas-free route, then return
logCalls = [];
await go("config", 300); // leaving logs triggers releaseLogs()
await go("logs", 700);   // return: fresh initial + auto-search with persisted INFO filter
const rowsC = box().childElementCount;
check("filters persisted across leave+return (rows=" + rowsC + ")", rowsC > 0 && Array.from(box().children).every((r) => r.textContent.includes("INFO") && !r.textContent.includes("DEBUG")));
check("re-entry issued a fresh latest-page fetch", logCalls.length > 0 && logCalls[0] === "initial");

console.log(fails === 0 ? "ALL PASS" : fails + " FAILURES");
process.exit(fails === 0 ? 0 : 1);