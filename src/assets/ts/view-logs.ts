// Logs tab: ring-buffer viewer with filters (level/logger/app/name/regex),
// infinite scroll backwards, export.
import { $, el, esc, snack } from "./core";
import { api } from "./state";

/* ============================= logs ============================= */

const LOG_PAGE = 300;
const ICON_FILTER = "M10 18h4v-2h-4v2zM3 6v2h18V6H3zm4 7h10v-2H7v2z";
const ICON_REFRESH =
  "M17.65 6.35C16.2 4.9 14.21 4 12 4c-4.42 0-7.99 3.58-7.99 8s3.57 8 7.99 8c3.73 0 6.84-2.55 7.73-6h-2.08c-.82 2.33-3.04 4-5.65 4-3.31 0-6-2.69-6-6s2.69-6 6-6c1.66 0 3.14.69 4.22 1.78L13 11h7V4l-2.35 2.35z";
const ICON_DOWNLOAD = "M19 9h-4V3H9v6H5l7 7 7-7zM5 18v2h14v-2H5z";

let logLoaded: any[] = []; // ring entries, oldest -> newest (all loaded pages)
let logTotal = 0;
let logOldestSeq = 0; // seq of the oldest loaded entry (0 = nothing loaded)
let logNoMore = false;
let logApps: string[] = [];
let logNames: { app: string; name: string }[] = [];
let logLoggers: string[] = [];
let logBusy = false;
let logSuggTimer = 0;
// bumped on every fresh base load / release so an in-flight fetchOlder that
// started before can detect its page is stale and discard it (gen guard).
let logGen = 0;
// OR within a category, AND across categories: e.g. (DEBUG or INFO) and (app A or app B).
const logFilters = {
  levels: [] as string[],
  loggers: [] as string[],
  apps: [] as string[],
  names: [] as string[],
  regex: "",
};
const LOG_CAT_KEYS: Record<string, string> = {
  level: "levels",
  logger: "loggers",
  app: "apps",
  name: "names",
};

function svgIcon(path: string, size = 16): SVGElement {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("width", String(size));
  svg.setAttribute("height", String(size));
  svg.setAttribute("fill", "currentColor");
  const p = document.createElementNS("http://www.w3.org/2000/svg", "path");
  p.setAttribute("d", path);
  svg.append(p);
  return svg;
}

function logRow(e: any): HTMLElement {
  const div = el("div", {}, [e.raw]);
  if (e.level === "WARN") div.className = "lwarn";
  else if (e.level === "ERROR") div.className = "lerror";
  return div;
}

function logMatches(e: any, re: RegExp | null): boolean {
  if (logFilters.levels.length && !logFilters.levels.includes(e.level)) return false;
  if (logFilters.loggers.length && !logFilters.loggers.includes(e.logger)) return false;
  if (logFilters.apps.length && !logFilters.apps.includes(e.app)) return false;
  if (logFilters.names.length && !logFilters.names.includes(e.name)) return false;
  if (re && !re.test(e.raw)) return false;
  return true;
}

function renderLogList(scrollBottom = false) {
  const box = $("#logs-box") as HTMLElement;
  const atBottom = box.scrollTop + box.clientHeight >= box.scrollHeight - 4;
  const st = box.scrollTop;
  box.innerHTML = "";
  let re: RegExp | null = null;
  if (logFilters.regex) {
    try {
      re = new RegExp(logFilters.regex);
    } catch {
      re = null;
    }
  }
  const frag = document.createDocumentFragment();
  for (const e of logLoaded) {
    if (logMatches(e, re)) frag.append(logRow(e));
  }
  box.append(frag);
  if (scrollBottom || atBottom) box.scrollTop = box.scrollHeight;
  else box.scrollTop = st;
  void ensureMatches();
}

/// Auto-paging: when the filtered view is empty (or thinner than a page) the
/// matching lines may simply live further back in the file-backed history
/// (e.g. DEBUG flooded the newest window). Keep pulling older pages until a
/// page's worth of rows is visible, history ends, or the scan cap is hit.
/// Same predicate as renderLogList (logMatches), so composed filters (OR
/// within a category, AND across, + regex) work unchanged.
const MAX_AUTO_PAGES = 40;
/// Cap on rows retained in the client ring (logLoaded + the DOM). Old pages
/// always live in the rotated on-disk files and can be re-fetched, so
/// dropping the oldest once a session pulls in too much history only bounds
/// memory — searches keep finding old lines, and RAM stops climbing forever.
const LOG_MAX = LOG_PAGE * (MAX_AUTO_PAGES + 2);
let logEnsureRunning = false;

function logStatus(msg: string) {
  const s = $("#logs-status");
  if (s) s.textContent = msg;
}

/// Drop the oldest retained rows (and their DOM rows via a re-render) once the
/// client holds more than LOG_MAX. Evicted pages are re-fetchable from the log
/// files, so this only prevents an unbounded session from pinning old pages in
/// memory; the newest entry is always preserved. Called with logBusy set (from
/// fetchOlder), so the nested renderLogList's ensureMatches bails — no recursion.
function pruneRetained() {
  const over = logLoaded.length - LOG_MAX;
  if (over <= 0) return;
  logLoaded.splice(0, over);
  if (logLoaded.length) {
    logOldestSeq = logLoaded[0].seq;
  } else {
    logOldestSeq = 0;
    logNoMore = true;
  }
  renderLogList();
}

/// Drop the client-side retained log ring and facet lists when the Logs tab is
/// left. The log store is on-disk and stateless (see state.rs LogFileSink), so
/// the retained pages are pure refetchable client cache — returning to the tab
/// re-fetches the newest page and re-runs the search, nothing is lost. The
/// filter state (logFilters) is deliberately kept: that is what restores the
/// user's active filtered view on return. Bumping logGen invalidates any
/// fetchOlder in flight so a stale older page can't repopulate the cleared
/// ring.
export function releaseLogs() {
  logLoaded = [];
  logTotal = 0;
  logOldestSeq = 0;
  logNoMore = true;
  logApps = [];
  logNames = [];
  logLoggers = [];
  clearTimeout(logSuggTimer);
  logGen++;
}

async function ensureMatches() {
  if (logEnsureRunning || logBusy || !$("#logs-box")) return;
  logEnsureRunning = true;
  try {
    const box = $("#logs-box") as HTMLElement;
    let pages = 0;
    while (box.childElementCount < LOG_PAGE && pages < MAX_AUTO_PAGES) {
      logStatus(pages ? `searching older logs… ${pages * LOG_PAGE}+ lines scanned` : "searching older logs…");
      const r = await fetchOlder();
      if (r < 0) break;
      pages++;
      if (logNoMore) break;
    }
    const exhausted = logNoMore || logOldestSeq <= 0;
    if (logTotal === 0) logStatus("no logs yet");
    else if (box.childElementCount === 0)
      logStatus(
        exhausted
          ? "no lines match the current filters"
          : `no matches in the last ${pages * LOG_PAGE} loaded lines — refine the filters (older lines live in the rotated log files)`,
      );
    else logStatus("");
  } finally {
    logEnsureRunning = false;
  }
}

function renderLogBadges() {
  const cont = $("#logs-fbadges");
  cont.innerHTML = "";
  const groups: [string, string, string[]][] = [
    ["Log level", "levels", logFilters.levels],
    ["Logger", "loggers", logFilters.loggers],
    ["App", "apps", logFilters.apps],
    ["Name", "names", logFilters.names],
  ];
  for (const [label, key, vals] of groups) {
    if (!vals.length) continue;
    const g = el("span", { class: "f-group" }, [label + ":"]);
    for (const v of vals) {
      const chip = el("span", { class: "f-chip" }, [v]);
      const x = el("button", { class: "f-x", title: "remove " + v }, ["✕"]);
      x.onclick = () => {
        (logFilters as any)[key] = vals.filter((y) => y !== v);
        renderLogBadges();
        renderLogList();
      };
      chip.append(x);
      g.append(chip);
    }
    cont.append(g);
  }
  if (logFilters.regex) {
    const g = el("span", { class: "f-group" }, ["Regex:"]);
    const chip = el("span", { class: "f-chip" }, ["/" + logFilters.regex + "/"]);
    const x = el("button", { class: "f-x", title: "remove regex" }, ["✕"]);
    x.onclick = () => {
      logFilters.regex = "";
      renderLogBadges();
      renderLogList();
    };
    chip.append(x);
    g.append(chip);
    cont.append(g);
  }
}

function toggleLogVal(key: string, v: string) {
  const arr = (logFilters as any)[key] as string[];
  const i = arr.indexOf(v);
  if (i >= 0) arr.splice(i, 1);
  else arr.push(v);
  renderLogBadges();
  renderLogList();
  renderLogSugg();
}

function suggChip(label: string, on: boolean, cb: () => void): HTMLElement {
  const c = el("button", { class: "sugg" + (on ? " on" : "") }, [(on ? "✓ " : "") + label]);
  c.onclick = cb;
  return c;
}

function renderLogSugg() {
  const cat = ($("#fl-cat") as HTMLSelectElement).value;
  const q = ($("#fl-val") as HTMLInputElement).value.trim().toLowerCase();
  const box = $("#fl-sugg");
  box.innerHTML = "";
  if (cat === "level") {
    for (const lv of ["DEBUG", "INFO", "WARN", "ERROR"]) {
      box.append(suggChip(lv, logFilters.levels.includes(lv), () => toggleLogVal("levels", lv)));
    }
    return;
  }
  if (cat === "regex") {
    box.append(el("div", { class: "muted" }, ["type a regex, press Enter to apply"]));
    return;
  }
  if (cat === "logger") {
    for (const l of logLoggers) {
      if (!q || l.toLowerCase().includes(q)) {
        box.append(suggChip(l, logFilters.loggers.includes(l), () => toggleLogVal("loggers", l)));
      }
    }
  } else if (cat === "app") {
    for (const a of logApps) {
      if (!q || a.toLowerCase().includes(q)) {
        box.append(suggChip(a, logFilters.apps.includes(a), () => toggleLogVal("apps", a)));
      }
    }
  } else {
    // names: narrowed by the selected apps (when any) + the typeahead text;
    // each suggestion shows its app so same-name ids stay distinguishable
    const selApps = logFilters.apps;
    let shown = 0;
    for (const p of logNames) {
      if (selApps.length && !selApps.includes(p.app)) continue;
      if (q && !p.name.toLowerCase().includes(q)) continue;
      const label = p.app ? p.name + "@" + p.app : p.name;
      box.append(suggChip(label, logFilters.names.includes(p.name), () => toggleLogVal("names", p.name)));
      shown++;
    }
    for (const n of logFilters.names) {
      if (!logNames.some((p) => p.name === n)) {
        box.append(suggChip(n, true, () => toggleLogVal("names", n)));
        shown++;
      }
    }
    if (!shown) box.append(el("div", { class: "muted" }, ["no names match"]));
  }
  if (cat !== "level" && cat !== "regex" && !box.childElementCount) {
    box.append(el("div", { class: "muted" }, ["no values yet — log some traffic first"]));
  }
}

function logsFetch(before?: number): Promise<any> {
  const q = new URLSearchParams({ limit: String(LOG_PAGE) });
  if (before !== undefined) q.set("before", String(before));
  return api("/logs?" + q.toString());
}

/// Load one older page and prepend its MATCHING rows. Returns the number of
/// rows added (0 = nothing added), or -1 when it could not run (busy,
/// exhausted, nothing loaded).
async function fetchOlder(): Promise<number> {
  if (logBusy || logNoMore || logOldestSeq <= 0) return -1;
  logBusy = true;
  const gen = logGen;
  const before = logOldestSeq;
  const box = $("#logs-box") as HTMLElement;
  const h0 = box.scrollHeight;
  let added = 0;
  try {
    const d = await logsFetch(before);
    if (gen !== logGen) return 0; // ring was cleared/loaded while this paged
    logTotal = d.total;
    if (d.lines.length === 0) {
      logNoMore = true;
      return 0;
    }
    logApps = d.apps;
    logNames = d.names;
    logLoggers = d.loggers;
    renderLogSugg();
    const seen = new Set(logLoaded.map((e: any) => e.seq));
    const fresh = d.lines.filter((e: any) => !seen.has(e.seq));
    if (fresh.length === 0) {
      logNoMore = true; // no progress possible (all duplicates) — stop paging
      return 0;
    }
    logLoaded = fresh.concat(logLoaded);
    logOldestSeq = logLoaded[0].seq;
    let re: RegExp | null = null;
    if (logFilters.regex) {
      try {
        re = new RegExp(logFilters.regex);
      } catch {
        re = null;
      }
    }
    const frag = document.createDocumentFragment();
    for (const e of fresh) {
      if (logMatches(e, re)) {
        frag.append(logRow(e));
        added++;
      }
    }
    box.insertBefore(frag, box.firstChild);
    box.scrollTop += box.scrollHeight - h0;
    pruneRetained();
    return added;
  } catch (e: any) {
    snack(e.message);
    return -1;
  } finally {
    logBusy = false;
  }
}

export async function renderLogs() {
  const v = $("#view");
  v.innerHTML = "";
  logLoaded = [];
  logTotal = 0;
  logOldestSeq = 0;
  logNoMore = false;
  logApps = [];
  logNames = [];
  logLoggers = [];
  v.append(
    el("div", { class: "card logs-card" }, [
      el("div", { class: "logs-head" }, [
        el("h3", {}, ["Server logs"]),
        el("div", { class: "row", style: "gap:8px" }, [
          el("button", { id: "logs-refresh", class: "btn btn-outline btn-small" }, [
            svgIcon(ICON_REFRESH, 14),
            " Refresh",
          ]),
          el("button", { id: "logs-export", class: "btn btn-outline btn-small" }, [
            svgIcon(ICON_DOWNLOAD, 14),
            " Download .txt",
          ]),
        ]),
      ]),
      el("div", { class: "logs-filterbar" }, [
        el(
          "button",
          {
            id: "logs-fbtn",
            class: "btn btn-outline btn-small",
            title: "Add log filters",
          },
          [svgIcon(ICON_FILTER, 14), " Add filter"],
        ),
        el("span", { id: "logs-fbadges", class: "logs-fbadges" }),
        el("span", {
          class: "muted",
          id: "logs-retention",
          style: "margin-left:auto",
        }),
        el("div", { class: "logs-pop", id: "logs-pop" }, [
          el("div", { class: "lp-row" }, [
            el("label", {}, ["Add"]),
            el("select", { id: "fl-cat" }, [
              el("option", { value: "level" }, ["log level"]),
              el("option", { value: "logger" }, ["logger"]),
              el("option", { value: "app" }, ["app id"]),
              el("option", { value: "name" }, ["name id"]),
              el("option", { value: "regex" }, ["regex"]),
            ]),
          ]),
          el("div", { class: "lp-row" }, [
            el("input", {
              id: "fl-val",
              type: "text",
              placeholder: "type to filter values…",
              spellcheck: "false",
            }),
          ]),
          el("div", { class: "fl-sugg", id: "fl-sugg" }),
          el(
            "div",
            {
              class: "row",
              style: "justify-content:space-between;margin-top:8px",
            },
            [
              el("span", { class: "muted", id: "fl-hint" }),
              el("button", { id: "fl-clear", class: "btn btn-outline btn-small" }, ["Clear all"]),
            ],
          ),
        ]),
      ]),
      el("div", {
        id: "logs-status",
        class: "muted",
        style: "padding:4px 14px 0",
      }),
      el("div", { class: "logs-box", id: "logs-box" }),
    ]),
  );

  const box = $("#logs-box") as HTMLElement;
  const load = async () => {
    logGen++; // any fetchOlder in flight is now stale — don't splice onto this fresh ring
    try {
      const d = await logsFetch();
      logLoaded = d.lines;
      logTotal = d.total;
      logOldestSeq = d.lines.length ? d.lines[0].seq : 0;
      logNoMore = false;
      logApps = d.apps;
      logNames = d.names;
      logLoggers = d.loggers;
      const r = d.retention;
      if (r && r.files) {
        $("#logs-retention").textContent =
          `${r.files} files × ${r.size_mb} MB (${r.path}) — set via log.files / log.size_mb in server.yml`;
      }
      renderLogSugg();
      renderLogList(true);
    } catch (e: any) {
      snack(e.message);
    }
  };
  box.addEventListener("scroll", () => {
    if (box.scrollTop < 40) void fetchOlder();
  });
  $("#logs-refresh").onclick = load;
  $("#logs-export").onclick = async () => {
    try {
      const d = await api("/logs"); // no params -> the full ring
      const blob = new Blob([d.lines.map((l: any) => l.raw).join("\n")], {
        type: "text/plain",
      });
      const a = el("a", {
        href: URL.createObjectURL(blob),
        download: "xavierdb.log",
      });
      a.click();
    } catch (e: any) {
      snack(e.message);
    }
  };
  // close when pressing outside the filter bar (incl. the popover).
  // mousedown + a select/option guard: opening the native <select> dropdown
  // must never close the popover (its popup events can escape contains()).
  document.addEventListener("mousedown", (ev) => {
    const t = ev.target as Node;
    if (t instanceof Element && t.closest("select, option")) return;
    const bar = $(".logs-filterbar");
    if (!bar || !bar.contains(t)) $("#logs-pop").classList.remove("open");
  });
  const pop = $("#logs-pop");
  const valIn = $("#fl-val") as HTMLInputElement;
  const setHint = () => {
    const cat = ($("#fl-cat") as HTMLSelectElement).value;
    const n = (logFilters as any)[LOG_CAT_KEYS[cat] || "regex"];
    const cnt = Array.isArray(n) ? n.length : n ? 1 : 0;
    $("#fl-hint").textContent = cnt ? cnt + " active" : "";
  };
  $("#logs-fbtn").addEventListener("mousedown", (ev) => ev.stopPropagation());
  $("#logs-fbtn").onclick = () => {
    pop.classList.toggle("open");
    if (pop.classList.contains("open")) {
      renderLogSugg();
      setHint();
      valIn.focus();
    }
  };
  $("#fl-cat").addEventListener("change", () => {
    valIn.value = "";
    valIn.placeholder =
      ($("#fl-cat") as HTMLSelectElement).value === "regex"
        ? "e.g. mongo|throttled (Enter to apply)"
        : "type to filter values…";
    renderLogSugg();
    setHint();
  });
  valIn.addEventListener("input", () => {
    clearTimeout(logSuggTimer);
    logSuggTimer = window.setTimeout(renderLogSugg, 120);
  });
  valIn.addEventListener("keydown", (ev) => {
    if (ev.key !== "Enter") return;
    const cat = ($("#fl-cat") as HTMLSelectElement).value;
    const v = valIn.value.trim();
    if (!v) return;
    if (cat === "regex") {
      logFilters.regex = v;
      renderLogBadges();
      renderLogList();
    } else {
      toggleLogVal(LOG_CAT_KEYS[cat], v);
    }
    valIn.value = "";
    renderLogSugg();
    setHint();
  });
  $("#fl-clear").onclick = () => {
    logFilters.levels = [];
    logFilters.loggers = [];
    logFilters.apps = [];
    logFilters.names = [];
    logFilters.regex = "";
    renderLogBadges();
    renderLogSugg();
    setHint();
    // back to the default Logs view: newest 300 lines unfiltered (the same
    // fresh page a first tab entry loads — load() also bumps logGen so any
    // in-flight older-page fetch is discarded rather than spliced onto it).
    load();
  };
  // Re-render the active filter chips after the DOM rebuild on each tab
  // entry: logFilters persists across route switches, so the previously set
  // badges must be painted back into the freshly-built #logs-fbadges.
  renderLogBadges();
  load();
}
