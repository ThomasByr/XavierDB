// Overview tab: stat chips, system mini-charts, the all-apps RPS chart
// (shared scale + selectable time window) and the top-apps traffic table.
import { $, el, esc, fmtNum, fmtBytes, fmtUptime, fmtRate } from "./core";
import { sparkline, drawMini, lineColor, getCss, withAlpha, emaSmooth } from "./charts";
import { rpsArchive, RPS_WINDOWS, getRpsWindowIdx, setRpsWindowIdx } from "./rps-archive";
import { updateMongoStatus } from "./mongo";
import { lastMetrics, Metrics, AppNode, systemHistory, getChartSmoothing } from "./state";
/* ============================= overview ============================= */

interface ChartDef {
  key: string;
  label: string;
  color: string;
  raw: (s: Metrics["system"]) => number;
  fmt: (s: Metrics["system"]) => string;
  sub: (s: Metrics["system"]) => string;
}

const chartDefs: ChartDef[] = [
  {
    key: "cpu",
    label: "CPU",
    color: "--primary",
    raw: (s) => s.cpu_pct,
    fmt: (s) => fmtNum(s.cpu_pct, 0) + "%",
    sub: () => "of all cores",
  },
  {
    key: "mem",
    label: "Memory",
    color: "--secondary",
    raw: (s) => s.mem_pct,
    fmt: (s) => fmtNum(s.mem_pct, 0) + "%",
    sub: (s) => `${fmtBytes(s.mem_used_mb)} / ${fmtBytes(s.mem_total_mb)}`,
  },
  {
    key: "disk",
    label: "Disk",
    color: "--warning",
    raw: (s) => s.disk_pct,
    fmt: (s) => fmtNum(s.disk_pct, 0) + "%",
    sub: (s) => `${fmtBytes(s.disk_used_mb)} / ${fmtBytes(s.disk_total_mb)}`,
  },
  {
    key: "rx",
    label: "Download",
    color: "--primary",
    raw: (s) => s.net_rx_kbps,
    fmt: (s) => fmtRate(s.net_rx_kbps),
    sub: () => "in",
  },
  {
    key: "tx",
    label: "Upload",
    color: "--secondary",
    raw: (s) => s.net_tx_kbps,
    fmt: (s) => fmtRate(s.net_tx_kbps),
    sub: () => "out",
  },
];
let chartEls: Record<string, { value: HTMLElement; canvas: HTMLCanvasElement; sub: HTMLElement }> = {};

export function renderOverview() {
  const v = $("#view");
  v.innerHTML = "";
  v.append(el("div", { class: "ov-alert hidden", id: "ov-alert" }));
  v.append(el("div", { class: "stats-row", id: "ov-chips" }));
  v.append(el("div", { class: "grid chart-grid", id: "ov-charts" }));
  chartEls = {};
  const grid = $("#ov-charts");
  for (const d of chartDefs) {
    const card = el("div", { class: "chart-card" });
    const head = el("div", { class: "chart-head" });
    head.append(el("span", { class: "chart-label" }, [d.label]));
    const value = el("span", { class: "chart-value" });
    head.append(value);
    const sub = el("div", { class: "chart-sub" });
    const canvas = el("canvas", {
      width: "200",
      height: "48",
    }) as HTMLCanvasElement;
    card.append(head, sub, canvas);
    grid.append(card);
    chartEls[d.key] = { value, canvas, sub };
  }
  v.append(
    el("div", { class: "card", id: "ov-rps" }, [
      el("h3", {}, [
        el("span", { id: "ov-rps-title" }, ["All apps · RPS"]),
        el("span", { class: "rps-mode", id: "ov-rps-mode" }, [
          el("span", { class: "rm-track" }, [
            el("span", { class: "rm-thumb" }),
            el("button", { class: "rm-opt", id: "ov-rps-mode-global", type: "button" }, ["Global"]),
            el("button", { class: "rm-opt", id: "ov-rps-mode-focus", type: "button" }, ["Focus"]),
          ]),
          el(
            "button",
            {
              class: "rm-arrow",
              id: "ov-rps-mode-arrow",
              type: "button",
              title: "choose the app plotted in Focus mode",
            },
            ["▾"],
          ),
        ]),
        el("span", {
          class: "muted",
          id: "ov-rps-summary",
          style: "font-weight:400",
        }),
      ]),
      el("div", { class: "rps-head" }, [
        el("div", { id: "ov-rps-legend", class: "rps-legend" }),
        el(
          "button",
          {
            id: "ov-rps-details",
            class: "btn btn-outline btn-small",
            title: "stacked per-name_id breakdown",
          },
          ["Show details"],
        ),
      ]),
      el("canvas", { id: "ov-rps-canvas", style: "width:100%;height:190px;display:block" }),
      el("div", { class: "rps-controls" }, [
        el(
          "button",
          { id: "ov-rps-win", class: "btn btn-outline btn-small", title: "horizontal axis window" },
          [RPS_WINDOWS[getRpsWindowIdx()][0]],
        ),
      ]),
    ]),
  );
  $("#ov-rps-win").onclick = () => openWinPop($("#ov-rps-win"));
  $("#ov-rps-details").onclick = () => {
    if (rpsMode !== "focus") openDetailsPop($("#ov-rps-details"));
  };
  $("#ov-rps-mode-global").onclick = () => switchRpsMode("global");
  $("#ov-rps-mode-focus").onclick = () => {
    // already in Focus with nothing selected: re-open the picker
    if (rpsMode === "focus" && !focusApp) toggleFocusPop();
    else switchRpsMode("focus");
  };
  $("#ov-rps-mode-arrow").onclick = () => toggleFocusPop();
  applyRpsModeUI();
  const rpsCanvas = $("#ov-rps-canvas") as HTMLCanvasElement;
  rpsCanvas.addEventListener("mousemove", onRpsHover);
  rpsCanvas.addEventListener("mouseleave", () => setRpsHover(null));
  rpsHoverT = null;
  v.append(
    el("div", { class: "card", id: "ov-traffic" }, [
      el("h3", {}, [
        "App traffic",
        el("span", {
          class: "muted",
          id: "ov-traffic-summary",
          style: "margin-left:auto;font-weight:400",
        }),
      ]),
      el("div", { id: "ov-traffic-body" }),
    ]),
  );
  if (lastMetrics) renderOverviewData(lastMetrics);
}

export function renderOverviewData(m: Metrics) {
  const s = m.system;
  const health = m.health.status === "ok";
  const chips = $("#ov-chips");
  if (chips) {
    chips.innerHTML = "";
    const cards: [string, string, string][] = [
      ["QPS", fmtNum(m.qps, 1), "requests / second"],
      ["Cursors", String(m.cursors.count), "active pagination"],
      [
        "MongoDB",
        health ? "reachable" : "DOWN",
        `ping ${fmtNum(m.health.mongodb?.ping_latency_ms ?? 0, 1)} ms`,
      ],
      ["Uptime", fmtUptime(s.uptime_s), "since server start"],
    ];
    for (const [label, value, sub] of cards) {
      const card = el("div", { class: "stat" });
      card.append(el("div", { class: "stat-label" }, [label]));
      card.append(el("div", { class: "stat-value" }, [value]));
      card.append(el("div", { class: "stat-sub" }, [sub]));
      chips.append(card);
    }
  }
  const record = (key: string, raw: number) => {
    const h = systemHistory[key];
    h.push(raw);
    if (h.length > 90) h.shift();
    return h;
  };
  for (const d of chartDefs) {
    const c = chartEls[d.key];
    if (!c) continue;
    c.value.textContent = d.fmt(s);
    c.sub.textContent = d.sub(s);
    drawMini(c.canvas, emaSmooth(record(d.key, d.raw(s)), getChartSmoothing()), getCss(d.color));
  }
  updateMongoStatus(m.health);
  renderOvAlert(m);
  renderOvTraffic(m);
  updateRpsChart(m);
}

/* blocked-apps alert strip — shown only while at least one app is blocked */
function renderOvAlert(m: Metrics) {
  const alert = $("#ov-alert");
  if (!alert) return;
  const blocked = m.apps.filter((a) => a.blocked);
  alert.classList.toggle("hidden", blocked.length === 0);
  alert.textContent = "";
  if (!blocked.length) return;
  alert.append(el("span", { style: "font-weight:600" }, ["Blocked apps"]));
  for (const a of blocked) {
    alert.append(
      el(
        "span",
        {
          class: "badge bad",
          title: `${esc(a.app)} — every request returns 403 BLOCKED`,
        },
        [esc(a.app)],
      ),
    );
  }
}

/* top apps by RPS + lifetime aggregate — rebuilt each poll like the limits table */
const OV_TOP_APPS = 6;

function renderOvTraffic(m: Metrics) {
  const body = $("#ov-traffic-body");
  if (!body) return;
  const summary = $("#ov-traffic-summary");
  const active = m.apps.filter((a) => a.rps > 0).sort((x, y) => y.rps - x.rps);
  const top = active.slice(0, OV_TOP_APPS);
  const sumRps = active.reduce((s, a) => s + a.rps, 0);
  const worstP50 = active.reduce((s, a) => Math.max(s, a.p50_ms), 0);
  const lifetime = m.health?.app?.total_requests ?? 0;
  if (summary) {
    summary.textContent =
      active.length === 0
        ? "no traffic yet"
        : `${active.length} active · ${fmtNum(sumRps, 1)} rps total · worst p50 ${fmtNum(worstP50, 1)}ms · ${fmtNum(lifetime, 0)} requests since server start`;
  }
  body.textContent = "";
  if (top.length === 0) {
    body.append(
      el("div", { class: "empty-note" }, ["no app traffic yet — apps appear here once they send requests"]),
    );
    return;
  }
  const t = el("table", { class: "data table-sm" });
  t.innerHTML =
    "<thead><tr><th>app</th><th>weight</th><th>trend</th><th>rps</th><th>p50</th><th>limit</th><th>status</th></tr></thead>";
  const tb = el("tbody");
  const rows = top.map((a) => ovTrafficRow(a));
  for (const r of rows) tb.append(r.tr);
  t.append(tb);
  body.append(t);
  // draw only once the canvases are laid out — clientWidth is 0 before attachment
  top.forEach((a, i) => sparkline(rows[i].canvas, a.rps_history, getCss("--primary")));
  if (active.length > top.length) {
    body.append(
      el("div", { class: "muted", style: "margin-top:6px" }, [
        `… and ${active.length - top.length} more app(s)`,
      ]),
    );
  }
}

function ovTrafficRow(a: AppNode): {
  tr: HTMLTableRowElement;
  canvas: HTMLCanvasElement;
} {
  const tr = el("tr");
  const name = el("td");
  name.append(el("b", {}, [esc(a.app)]));
  if (a.blocked) name.append(el("span", { class: "badge bad", style: "margin-left:6px" }, ["BLOCKED"]));
  tr.append(name);
  tr.append(el("td", { class: "tnum" }, ["×" + (a.weight ?? 1).toFixed(1)]));
  const cv = el("canvas", {
    class: "sparkline",
    width: "70",
    height: "22",
    style: "width:70px;height:22px",
  }) as HTMLCanvasElement;
  const trend = el("td");
  trend.append(cv);
  tr.append(trend);
  tr.append(el("td", { class: "tnum" }, [fmtNum(a.rps, 1)]));
  tr.append(el("td", { class: "tnum" }, [fmtNum(a.p50_ms, 1)]));
  tr.append(el("td", { class: "tnum" }, [String(a.limit ?? "—")]));
  tr.append(
    el("td", {}, [
      el("span", { class: "badge " + (a.blocked ? "bad" : "ok") }, [a.blocked ? "BLOCKED" : "active"]),
    ]),
  );
  return { tr, canvas: cv };
}

/* ---- overview: all-apps RPS chart (shared y scale, selectable window) ---- */

function fmtAxisTime(ms: number, windowSec: number): string {
  const d = new Date(ms);
  if (windowSec <= 2 * 86400) return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (windowSec <= 100 * 86400) return d.toLocaleDateString([], { month: "short", day: "numeric" });
  return d.toLocaleDateString([], { year: "2-digit", month: "short" });
}

/* linear interpolation of a series at time t (clamped at both ends) */
function interp(pts: { t: number; v: number }[], t: number): number {
  if (!pts.length) return 0;
  if (t <= pts[0].t) return pts[0].v;
  const last = pts[pts.length - 1];
  if (t >= last.t) return last.v;
  for (let i = 1; i < pts.length; i++) {
    if (pts[i].t >= t) {
      const a = pts[i - 1];
      const b = pts[i];
      const f = (t - a.t) / Math.max(1e-9, b.t - a.t);
      return a.v + (b.v - a.v) * f;
    }
  }
  return last.v;
}

interface RpsSeries {
  app: string;
  color: string;
  pts: { t: number; v: number }[];
}

/* one expanded app: its name_ids ranked by contribution (biggest first) and
   the cumulative levels of the stack — cum[i][k] = sum of names 0..i at
   times[k], so cum[n-1] ≈ the app line (drawn separately at full opacity) */
interface NameStack {
  app: string;
  color: string;
  /** drawn bands, biggest contributor first; the last entry may be the
   *  synthetic hatched "others" aggregate (see othersIdx) */
  names: string[];
  times: number[];
  cum: number[][];
  /** index of the merged "others" band in names, -1 when there is none */
  othersIdx: number;
  /** one point-series per drawn band (kept name_ids, biggest first, plus
   *  the synthetic "others" aggregate) — hover rows mirror the chart */
  bandPts: { t: number; v: number }[][];
}

/* which apps are expanded into a stacked name_id breakdown (persisted) */
const RPS_DETAILS_LS = "xdb-rps-details";
let detailApps: Set<string> = new Set(
  (() => {
    try {
      const d = JSON.parse(localStorage.getItem(RPS_DETAILS_LS) ?? "[]");
      return Array.isArray(d) ? d.filter((x) => typeof x === "string") : [];
    } catch {
      return [];
    }
  })(),
);
function saveDetailApps() {
  localStorage.setItem(RPS_DETAILS_LS, JSON.stringify([...detailApps]));
}

/* chart mode (persisted, per-client): "global" = one line per app (the
   classic view), "focus" = one line per name_id of a single app picked
   with the ▾ selector next to the Focus option */
type RpsMode = "global" | "focus";
const RPS_MODE_LS = "xdb-rps-mode";
let rpsMode: RpsMode = (() => {
  try {
    return localStorage.getItem(RPS_MODE_LS) === "focus" ? "focus" : "global";
  } catch {
    return "global";
  }
})();
function setRpsMode(m: RpsMode) {
  rpsMode = m;
  localStorage.setItem(RPS_MODE_LS, m);
}

/* the app whose name_id series are plotted in Focus mode (persisted) */
const RPS_FOCUS_LS = "xdb-rps-focus";
let focusApp: string | null = (() => {
  try {
    return localStorage.getItem(RPS_FOCUS_LS) || null;
  } catch {
    return null;
  }
})();
function setFocusApp(app: string) {
  focusApp = app;
  localStorage.setItem(RPS_FOCUS_LS, app);
}

/* mirror the mode into the segmented control, the card title and the
   "Show details" button (stacked breakdown is a Global-mode feature —
   Focus already plots one line per name_id) */
function applyRpsModeUI() {
  const modeEl = $("#ov-rps-mode");
  if (modeEl) modeEl.setAttribute("data-mode", rpsMode);
  const globalBtn = $("#ov-rps-mode-global");
  if (globalBtn) globalBtn.classList.toggle("on", rpsMode === "global");
  const focusBtn = $("#ov-rps-mode-focus");
  if (focusBtn) focusBtn.classList.toggle("on", rpsMode === "focus");
  const title = $("#ov-rps-title");
  if (title) {
    if (rpsMode === "focus" && focusApp) {
      title.textContent = `${focusApp} · RPS`;
      title.setAttribute("title", focusApp);
    } else {
      title.textContent = "All apps · RPS";
      title.removeAttribute("title");
    }
  }
  const dbtn = $("#ov-rps-details");
  if (dbtn) {
    if (rpsMode === "focus") {
      dbtn.setAttribute("disabled", "");
      dbtn.setAttribute("title", "Focus mode already plots every name_id — switch to Global first");
    } else {
      dbtn.removeAttribute("disabled");
      dbtn.setAttribute("title", "stacked per-name_id breakdown");
    }
  }
}

function switchRpsMode(m: RpsMode) {
  if (rpsMode === m) return;
  closeWinPop();
  closeDetailsPop();
  closeFocusPop();
  setRpsMode(m);
  applyRpsModeUI();
  if (m === "focus" && !focusApp) openFocusPop(); // nothing saved yet: guide the pick
  if (lastMetrics) updateRpsChart(lastMetrics);
}

/* contribution threshold (% of an app's window-average rps): name_ids
   under it merge into one hatched "others" band. Default 33 → at most 3
   individual bands. 0 = show every band. Persisted. */
const RPS_THRESHOLD_LS = "xdb-rps-threshold";
const RPS_THRESHOLD_DEFAULT = 33;
let rpsThreshold: number = (() => {
  try {
    const v = parseInt(localStorage.getItem(RPS_THRESHOLD_LS) ?? "", 10);
    return Number.isFinite(v) && v >= 0 && v <= 100 ? v : RPS_THRESHOLD_DEFAULT;
  } catch {
    return RPS_THRESHOLD_DEFAULT;
  }
})();
function setRpsThreshold(v: number) {
  rpsThreshold = Math.min(100, Math.max(0, Math.round(v)));
  localStorage.setItem(RPS_THRESHOLD_LS, String(rpsThreshold));
}

function buildNameStacks(m: Metrics, apps: string[], win: number, nowSec: number): NameStack[] {
  const stacks: NameStack[] = [];
  for (const app of apps) {
    if (!detailApps.has(app)) continue;
    const a = m.apps.find((x) => x.app === app);
    if (!a || !a.names.length) continue;
    const keys = a.names.map((n) => ({ name: n.name, key: `name:${n.name}@${app}`, rps: n.rps }));
    const data = rpsArchive.window(
      keys.map((k) => k.key),
      win,
      nowSec,
    );
    // rank by contribution over the displayed window (fallback: live rps)
    const ranked = keys
      .map((k) => {
        const pts = data.get(k.key) ?? [];
        let sum = 0;
        for (const p of pts) sum += p.v;
        return { name: k.name, pts, contrib: Math.max(pts.length ? sum / pts.length : 0, k.rps) };
      })
      .filter((k) => k.contrib > 0)
      .sort((x, y) => y.contrib - x.contrib);
    if (!ranked.length) continue;
    const times = Array.from(new Set(ranked.flatMap((r) => r.pts.map((p) => p.t)))).sort((x, y) => x - y);
    if (!times.length) continue;
    // split: names reaching the threshold share of the app's window total
    // keep their own band, the rest merge into one synthetic "others" band
    // (the top contributor is always kept, even above a 50% threshold)
    const total = ranked.reduce((s, r) => s + r.contrib, 0);
    let keep = ranked.length;
    if (rpsThreshold > 0 && total > 0)
      keep = Math.max(1, ranked.filter((r) => (r.contrib / total) * 100 >= rpsThreshold - 1e-9).length);
    const bands = ranked.slice(0, keep).map((r) => ({
      name: r.name,
      vals: times.map((t) => interp(r.pts, t)),
    }));
    let othersIdx = -1;
    const rest = ranked.slice(keep);
    if (rest.length) {
      othersIdx = bands.length;
      bands.push({
        name: `others (${rest.length})`,
        vals: times.map((t) => rest.reduce((s, r) => s + interp(r.pts, t), 0)),
      });
    }
    const cum: number[][] = [];
    for (let i = 0; i < bands.length; i++)
      cum.push(times.map((_, k) => (i === 0 ? bands[i].vals[k] : cum[i - 1][k] + bands[i].vals[k])));
    stacks.push({
      app,
      color: lineColor(app),
      names: bands.map((b) => b.name),
      times,
      cum,
      othersIdx,
      bandPts: bands.map((b) => b.vals.map((v, k) => ({ t: times[k], v }))),
    });
  }
  return stacks;
}

/* EMA over the y-values of a {t, v} series, keeping timestamps */
function smoothPts(pts: { t: number; v: number }[], a: number): { t: number; v: number }[] {
  const vs = emaSmooth(
    pts.map((p) => p.v),
    a,
  );
  return pts.map((p, i) => ({ t: p.t, v: vs[i] }));
}

function updateRpsChart(m: Metrics) {
  const canvas = $("#ov-rps-canvas") as HTMLCanvasElement | null;
  if (!canvas) return;
  const nowMs = Date.now();
  const [, win] = RPS_WINDOWS[getRpsWindowIdx()];
  const nowSec = Math.floor(nowMs / 1000);
  // Global: one series per app (+ optional stacked name_id breakdowns).
  // Focus: one series per name_id of the selected app — same archive, same
  // window, same shared scale, just keyed on "name:<id>@<app>".
  const series: RpsSeries[] = [];
  let stacks: NameStack[] = [];
  const cur = new Map<string, number>();
  if (rpsMode === "focus") {
    const node = m.apps.find((x) => x.app === focusApp);
    const names = (node?.names ?? []).map((n) => n.name).sort();
    const data = rpsArchive.window(
      names.map((n) => `name:${n}@${focusApp}`),
      win,
      nowSec,
    );
    for (const n of names) {
      series.push({ app: n, color: lineColor(n), pts: data.get(`name:${n}@${focusApp}`) ?? [] });
      cur.set(n, node?.names.find((x) => x.name === n)?.rps ?? 0);
    }
  } else {
    const apps = m.apps.map((a) => a.app).sort();
    const data = rpsArchive.window(apps, win, nowSec);
    for (const app of apps) {
      series.push({ app, color: lineColor(app), pts: data.get(app) ?? [] });
      cur.set(app, m.apps.find((x) => x.app === app)?.rps ?? 0);
    }
    stacks = buildNameStacks(m, apps, win, nowSec);
  }
  /* TensorBoard-style EMA applied at draw time over each plotted series
     (app lines, stacked cumulative bands + their per-band outlines — linear
     filter, so smoothing the cum rows ≡ cumulating the smoothed bands and
     the stack still sums to the smoothed app line). a=0 = raw. */
  const alpha = getChartSmoothing();
  if (alpha > 0) {
    for (const s of series) s.pts = smoothPts(s.pts, alpha);
    for (const st of stacks) {
      st.cum = st.cum.map((row) => emaSmooth(row, alpha));
      st.bandPts = st.bandPts.map((band) => smoothPts(band, alpha));
    }
  }
  let peak = 0;
  for (const s of series) for (const p of s.pts) if (p.v > peak) peak = p.v;
  const legend = $("#ov-rps-legend");
  if (legend) {
    legend.textContent = "";
    for (const s of series) {
      legend.append(
        el("span", { class: "rl", title: esc(s.app) }, [
          el("i", { style: "background:" + s.color }),
          esc(s.app) + " · " + fmtNum(cur.get(s.app) ?? 0, 1),
        ]),
      );
    }
  }
  const summary = $("#ov-rps-summary");
  if (summary) {
    const sinceMs = rpsArchive.startSec * 1000;
    const partial = sinceMs > 0 && sinceMs > nowMs - win * 1000;
    let text: string;
    if (rpsMode === "focus" && !series.length) {
      text = focusApp ? `${focusApp} — no name_id series yet` : "no app selected — pick one with the ▾ arrow";
    } else {
      const unit = rpsMode === "focus" ? "name_id(s)" : "app(s)";
      text =
        `${series.length} ${unit} · shared scale · peak ${fmtNum(peak, 1)} rps` +
        (partial ? ` · collecting since ${fmtAxisTime(sinceMs, win)}` : "");
    }
    summary.textContent = text;
  }
  const btn = $("#ov-rps-win");
  if (btn) btn.textContent = RPS_WINDOWS[getRpsWindowIdx()][0];
  const dbtn = $("#ov-rps-details");
  if (dbtn) dbtn.textContent = detailApps.size ? `Show details · ${detailApps.size}` : "Show details";
  rpsDrawArgs = { canvas, series, stacks, windowSec: win, nowMs };
  drawAppRpsChart(canvas, series, stacks, win, nowMs, rpsHoverT);
}

const RPS_ML = 46,
  RPS_MR = 10,
  RPS_MT = 8,
  RPS_MB = 20;

/* diagonal-hatch fill pattern for the merged "others" band — visually
   distinct from the real name bands. Cached per app color (the stacks are
   only ever drawn on the single RPS chart canvas, so the patterns are
   safely reused across redraws). */
const hatchCache = new Map<string, CanvasPattern>();
function hatchPattern(ctx: CanvasRenderingContext2D, color: string): CanvasPattern | string {
  let p = hatchCache.get(color);
  if (!p) {
    const c = el("canvas", { width: "8", height: "8" }) as HTMLCanvasElement;
    const hc = c.getContext("2d");
    if (!hc) return withAlpha(color, 0.15); // no 2d context: plain translucent fill
    hc.strokeStyle = withAlpha(color, 0.35);
    hc.lineWidth = 1.2;
    hc.beginPath();
    hc.moveTo(-2, 2);
    hc.lineTo(2, -2);
    hc.moveTo(-2, 10);
    hc.lineTo(10, -2);
    hc.moveTo(6, 14);
    hc.lineTo(14, 6);
    hc.stroke();
    p = ctx.createPattern(c, "repeat") ?? undefined;
    if (p) hatchCache.set(color, p);
    else return withAlpha(color, 0.15);
  }
  return p;
}

function drawAppRpsChart(
  canvas: HTMLCanvasElement,
  series: RpsSeries[],
  stacks: NameStack[],
  windowSec: number,
  nowMs: number,
  hoverSec: number | null,
) {
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth || 600;
  const h = canvas.clientHeight || 190;
  canvas.width = w * dpr;
  canvas.height = h * dpr;
  const ctx = canvas.getContext("2d")!;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  const ml = RPS_ML,
    mr = RPS_MR,
    mt = RPS_MT,
    mb = RPS_MB;
  const iw = w - ml - mr,
    ih = h - mt - mb;
  const t1 = nowMs / 1000,
    t0 = t1 - windowSec;
  let vmax = 1e-9;
  for (const s of series) for (const p of s.pts) if (p.v > vmax) vmax = p.v;
  vmax = Math.max(vmax * 1.08, 1); // shared scale: one y-axis for every app
  const grid = getCss("--outline-variant");
  const txt = getCss("--on-surface-variant");
  ctx.font = "10px system-ui, sans-serif";
  // horizontal grid + y labels
  const divs = 4;
  for (let i = 0; i <= divs; i++) {
    const y = mt + ih - (i / divs) * ih;
    ctx.strokeStyle = grid;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(ml, Math.round(y) + 0.5);
    ctx.lineTo(ml + iw, Math.round(y) + 0.5);
    ctx.stroke();
    ctx.fillStyle = txt;
    ctx.textAlign = "right";
    ctx.fillText(fmtNum((vmax * i) / divs, 1), ml - 6, y + 3);
  }
  // x labels
  ctx.textAlign = "center";
  for (let i = 0; i <= 4; i++) {
    const frac = i / 4;
    ctx.fillStyle = txt;
    ctx.fillText(fmtAxisTime((t0 + frac * windowSec) * 1000, windowSec), ml + frac * iw, h - 6);
  }
  const xPix = (t: number) => ml + ((t - t0) / windowSec) * iw;
  const yPix = (v: number) => mt + ih - (Math.min(v, vmax) / vmax) * ih;

  // stacked name_id breakdown UNDER the app lines: filled bands between
  // cumulative levels (band i = names[i]), biggest contributor at the bottom
  // with the least transparency, growing transparency going up. The top
  // level is NOT stroked — it is the app line, drawn below at full opacity.
  for (const st of stacks) {
    const n = st.names.length;
    // most contributing band: 50% opacity, fading to 10% going up the stack
    const bandA = (i: number) => (n <= 1 ? 0.5 : 0.5 - (0.4 * i) / (n - 1));
    const lineA = (i: number) => (n <= 2 ? 0.85 : 0.85 - (0.55 * i) / (n - 2));
    for (let i = 0; i < n; i++) {
      ctx.beginPath();
      for (let k = 0; k < st.times.length; k++) {
        const x = xPix(st.times[k]),
          y = yPix(st.cum[i][k]);
        k === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
      }
      if (i > 0) {
        for (let k = st.times.length - 1; k >= 0; k--) ctx.lineTo(xPix(st.times[k]), yPix(st.cum[i - 1][k]));
      } else {
        ctx.lineTo(xPix(st.times[st.times.length - 1]), mt + ih);
        ctx.lineTo(xPix(st.times[0]), mt + ih);
      }
      ctx.closePath();
      ctx.fillStyle = st.othersIdx === i ? hatchPattern(ctx, st.color) : withAlpha(st.color, bandA(i));
      ctx.fill();
    }
    ctx.lineWidth = 1.4;
    ctx.lineJoin = "round";
    for (let i = 0; i < n - 1; i++) {
      ctx.beginPath();
      for (let k = 0; k < st.times.length; k++) {
        const x = xPix(st.times[k]),
          y = yPix(st.cum[i][k]);
        k === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
      }
      ctx.strokeStyle = withAlpha(st.color, lineA(i));
      ctx.stroke();
    }
  }

  // one line per app, all on the shared scale (x = real time — gaps compress)
  for (const s of series) {
    if (s.pts.length < 1) continue;
    ctx.beginPath();
    let started = false;
    for (const p of s.pts) {
      const x = xPix(p.t);
      const y = yPix(p.v);
      if (!started) {
        ctx.moveTo(x, y);
        started = true;
      } else ctx.lineTo(x, y);
    }
    ctx.strokeStyle = s.color;
    ctx.lineWidth = 1.6;
    ctx.lineJoin = "round";
    ctx.stroke();
  }

  drawNameLabels(ctx, stacks, xPix, yPix, w, ml, iw);
  if (hoverSec != null && hoverSec >= t0 && hoverSec <= t1)
    drawRpsHover(ctx, w, h, ml, mt, mb, iw, ih, t0, windowSec, series, stacks, hoverSec);
}

/* name_id labels at the right edge — each right below its own cumulative
   line (kept ≥12px apart when lines are close together) */
function drawNameLabels(
  ctx: CanvasRenderingContext2D,
  stacks: NameStack[],
  xPix: (t: number) => number,
  yPix: (v: number) => number,
  h: number,
  ml: number,
  iw: number,
) {
  if (!stacks.length) return;
  const labels: { text: string; y: number; color: string }[] = [];
  for (const st of stacks) {
    const last = st.times.length - 1;
    for (let i = 0; i < st.names.length; i++)
      labels.push({ text: st.names[i], y: yPix(st.cum[i][last]), color: st.color });
  }
  labels.sort((a, b) => b.y - a.y); // bottom first
  ctx.save();
  ctx.beginPath();
  ctx.rect(ml, 0, iw + RPS_MR + 2, h);
  ctx.clip();
  ctx.font = "9.5px ui-monospace, Consolas, monospace";
  ctx.textAlign = "right";
  const maxW = Math.max(60, Math.min(170, iw * 0.45));
  let prevY = Infinity;
  for (const lb of labels) {
    let text = lb.text;
    while (text.length > 1 && ctx.measureText(text).width > maxW) text = text.slice(0, -1);
    if (text !== lb.text) text += "…";
    let ly = lb.y + 11;
    if (prevY !== Infinity && ly > prevY - 12) ly = prevY - 12;
    prevY = ly;
    ctx.fillStyle = withAlpha(lb.color, 0.95);
    ctx.fillText(text, ml + iw - 2, ly);
  }
  ctx.restore();
}

/* ---- hover: dashed crosshair + light tooltip (Chart.js-style) ---- */

function rr(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

function drawRpsHover(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  ml: number,
  mt: number,
  mb: number,
  iw: number,
  ih: number,
  t0: number,
  windowSec: number,
  series: RpsSeries[],
  stacks: NameStack[],
  t: number,
) {
  const x = ml + ((t - t0) / windowSec) * iw;
  // vertical crosshair at the hovered time
  ctx.strokeStyle = withAlpha(getCss("--on-surface-variant"), 0.6);
  ctx.lineWidth = 1;
  ctx.setLineDash([3, 3]);
  ctx.beginPath();
  ctx.moveTo(Math.round(x) + 0.5, mt);
  ctx.lineTo(Math.round(x) + 0.5, mt + ih);
  ctx.stroke();
  ctx.setLineDash([]);

  // rows: every app, name_ids nested under expanded ones
  interface Row {
    app: boolean;
    label: string;
    value: number;
    color: string;
  }
  const natural: Row[] = [];
  for (const s of series) {
    natural.push({ app: true, label: s.app, value: interp(s.pts, t), color: s.color });
    const st = stacks.find((x2) => x2.app === s.app);
    if (st)
      for (let i = 0; i < st.names.length; i++)
        natural.push({ app: false, label: st.names[i], value: interp(st.bandPts[i], t), color: st.color });
  }
  /* Hover rows re-ordered by the RPS at the cursor (highest on top). The
     legend keeps its alphabetical order — only this panel is sorted.
     Global: app blocks move as whole units so a details app keeps its
     nested name_id rows in their natural chart order. Focus: the flat
     name_id rows sort directly. */
  const rows: Row[] =
    rpsMode === "focus"
      ? [...natural].sort((a, b) => b.value - a.value)
      : (() => {
          const blocks: { app: Row; names: Row[] }[] = [];
          for (const r of natural) {
            if (r.app) blocks.push({ app: r, names: [] });
            else blocks[blocks.length - 1]?.names.push(r);
          }
          blocks.sort((a, b) => b.app.value - a.app.value);
          return blocks.flatMap((b) => [b.app, ...b.names]);
        })();
  const vals = rows.map((r) => fmtNum(r.value, 1));

  // measure + place the panel (flip to the left near the right edge)
  const pad = 8,
    rowH = 14,
    headH = 16,
    nameIndent = 28;
  ctx.font = "10.5px system-ui, sans-serif";
  let labelW = 0;
  for (const r of rows) labelW = Math.max(labelW, ctx.measureText(r.label).width);
  ctx.font = "10.5px ui-monospace, Consolas, monospace";
  let valW = 0;
  for (const v of vals) valW = Math.max(valW, ctx.measureText(v).width);
  const tw = pad * 2 + nameIndent + labelW + 10 + valW;
  const th = pad * 2 + headH + rows.length * rowH;
  let px = x + 12;
  if (px + tw > w - 4) px = x - 12 - tw;
  let py = mt + 4;
  if (py + th > h - mb) py = Math.max(mt, h - mb - th);

  rr(ctx, px, py, tw, th, 8);
  ctx.fillStyle = withAlpha(getCss("--surface"), 0.94);
  ctx.fill();
  ctx.strokeStyle = getCss("--outline-variant");
  ctx.lineWidth = 1;
  ctx.stroke();

  ctx.fillStyle = getCss("--on-surface-variant");
  ctx.font = "10.5px system-ui, sans-serif";
  ctx.textAlign = "left";
  ctx.fillText(fmtAxisTime(t * 1000, windowSec), px + pad, py + pad + 9);

  // vertical app-color bar spanning each expanded app row + its name rows
  for (let i = 0; i < rows.length; i++) {
    if (!rows[i].app) continue;
    let j = i;
    while (j + 1 < rows.length && !rows[j + 1].app) j++;
    if (j > i) {
      const y0 = py + pad + headH + i * rowH + 2;
      const y1 = py + pad + headH + j * rowH + rowH - 2;
      ctx.fillStyle = withAlpha(rows[i].color, 0.65);
      ctx.fillRect(px + pad + 12, y0, 2, y1 - y0);
    }
  }
  for (let i = 0; i < rows.length; i++) {
    const r = rows[i];
    const top = py + pad + headH + i * rowH;
    if (r.app) {
      ctx.fillStyle = r.color;
      ctx.fillRect(px + pad, top + 3, 8, 8);
    }
    ctx.fillStyle = getCss(r.app ? "--on-surface" : "--on-surface-variant");
    ctx.font = (r.app ? "600 " : "") + "10.5px system-ui, sans-serif";
    ctx.textAlign = "left";
    ctx.fillText(r.label, px + pad + (r.app ? 17 : nameIndent), top + 10);
    ctx.font = "10.5px ui-monospace, Consolas, monospace";
    ctx.fillStyle = getCss("--on-surface");
    ctx.textAlign = "right";
    ctx.fillText(vals[i], px + tw - pad, top + 10);
  }
}

/* hover state — cached draw args let mousemove redraw without a re-poll */
let rpsDrawArgs: {
  canvas: HTMLCanvasElement;
  series: RpsSeries[];
  stacks: NameStack[];
  windowSec: number;
  nowMs: number;
} | null = null;
let rpsHoverT: number | null = null;

function setRpsHover(t: number | null) {
  if (rpsHoverT === t) return;
  rpsHoverT = t;
  const d = rpsDrawArgs;
  if (d) drawAppRpsChart(d.canvas, d.series, d.stacks, d.windowSec, d.nowMs, t);
}

function onRpsHover(ev: MouseEvent) {
  const d = rpsDrawArgs;
  if (!d) return;
  const canvas = ev.currentTarget as HTMLCanvasElement;
  const rect = canvas.getBoundingClientRect();
  const x = ev.clientX - rect.left;
  const iw = rect.width - RPS_ML - RPS_MR;
  if (iw <= 0 || x < RPS_ML || x > RPS_ML + iw) {
    setRpsHover(null);
    return;
  }
  setRpsHover(d.nowMs / 1000 - d.windowSec + ((x - RPS_ML) / iw) * d.windowSec);
}

/* window-selector popover under the chart (weight-pop pattern) */
let winPopEl: HTMLElement | null = null;
let winPopDoc = false;

function closeWinPop() {
  winPopEl?.remove();
  winPopEl = null;
  if (winPopDoc) {
    document.removeEventListener("mousedown", winPopDocHandler);
    winPopDoc = false;
  }
}
function winPopDocHandler(ev: MouseEvent) {
  if (winPopEl && !winPopEl.contains(ev.target as Node)) closeWinPop();
}

function openWinPop(btn: HTMLElement) {
  closeWinPop();
  closeDetailsPop();
  closeFocusPop();
  const pop = el("div", { class: "win-pop" });
  const row = el("div", { class: "wp-row" });
  const val = el("span", { class: "wp-val" }, [RPS_WINDOWS[getRpsWindowIdx()][0]]);
  row.append(el("span", { class: "wp-title" }, ["time window"]), val);
  const slider = el("input", {
    type: "range",
    min: "0",
    max: String(RPS_WINDOWS.length - 1),
    step: "1",
    value: String(getRpsWindowIdx()),
  }) as HTMLInputElement;
  slider.addEventListener("input", () => {
    setRpsWindowIdx(parseInt(slider.value, 10));
    val.textContent = RPS_WINDOWS[getRpsWindowIdx()][0];
    btn.textContent = RPS_WINDOWS[getRpsWindowIdx()][0];
    if (lastMetrics) updateRpsChart(lastMetrics);
  });
  pop.append(row, slider);
  pop.append(
    el("div", { class: "wp-hint" }, ["1 minute → 1 year · history is sampled while the dashboard is open"]),
  );
  pop.addEventListener("mousedown", (e) => e.stopPropagation());
  (btn.parentElement as HTMLElement).appendChild(pop);
  winPopEl = pop;
  document.addEventListener("mousedown", winPopDocHandler);
  winPopDoc = true;
  slider.focus();
}

/* "Show details" popover: per-app switches for the stacked name_id
   breakdown (multi-select, persisted) */
let detPopEl: HTMLElement | null = null;
let detPopDoc = false;

function closeDetailsPop() {
  detPopEl?.remove();
  detPopEl = null;
  if (detPopDoc) {
    document.removeEventListener("mousedown", detPopDocHandler);
    detPopDoc = false;
  }
}
function detPopDocHandler(ev: MouseEvent) {
  if (detPopEl && !detPopEl.contains(ev.target as Node)) closeDetailsPop();
}

function openDetailsPop(btn: HTMLElement) {
  if (detPopEl) {
    closeDetailsPop();
    return;
  }
  closeWinPop();
  closeFocusPop();
  const pop = el("div", { class: "det-pop" });
  pop.append(el("div", { class: "dp-title" }, ["name_id breakdown"]));
  const list = el("div", { class: "dp-list" });
  const apps = (lastMetrics ? mApps(lastMetrics) : [...detailApps]).sort();
  if (!apps.length)
    list.append(el("div", { class: "wp-hint" }, ["no apps yet — they appear once they send requests"]));
  for (const app of apps) {
    const row = el("label", { class: "dp-row", title: esc(app) });
    const cb = el("input", { type: "checkbox" }) as HTMLInputElement;
    cb.checked = detailApps.has(app);
    cb.addEventListener("change", () => {
      if (cb.checked) detailApps.add(app);
      else detailApps.delete(app);
      saveDetailApps();
      if (lastMetrics) updateRpsChart(lastMetrics);
    });
    row.append(
      cb,
      el("span", { class: "dp-sw", style: "background:" + lineColor(app) }),
      el("span", { class: "dp-name" }, [esc(app)]),
    );
    list.append(row);
  }
  pop.append(list);
  // contribution threshold: name_ids under this % of an app's window
  // traffic merge into one hatched "others" band (0 = show every band)
  const thrRow = el("div", { class: "dp-thr-row" });
  const thrVal = el("span", { class: "wp-val" }, [`≥ ${rpsThreshold}%`]);
  thrRow.append(el("span", { class: "wp-title" }, ["name_id threshold"]), thrVal);
  const thrSlider = el("input", {
    type: "range",
    min: "0",
    max: "100",
    step: "1",
    value: String(rpsThreshold),
  }) as HTMLInputElement;
  thrSlider.addEventListener("input", () => {
    setRpsThreshold(parseInt(thrSlider.value, 10));
    thrVal.textContent = `≥ ${rpsThreshold}%`;
    if (lastMetrics) updateRpsChart(lastMetrics);
  });
  pop.append(thrRow, thrSlider);
  pop.append(
    el("div", { class: "wp-hint" }, [
      "stacks each name_id of the selected app(s) · the app line tops the stack",
      el("br"),
      'name_ids under the threshold share merge into a hatched "others" band · top contributor always shown',
    ]),
  );
  pop.addEventListener("mousedown", (e) => e.stopPropagation());
  (btn.parentElement as HTMLElement).appendChild(pop);
  detPopEl = pop;
  document.addEventListener("mousedown", detPopDocHandler);
  detPopDoc = true;
}

function mApps(m: Metrics): string[] {
  const ids = new Set(m.apps.map((a) => a.app));
  for (const a of detailApps) ids.add(a); // keep selections for apps not live right now
  return [...ids];
}

/* Focus-mode app picker (single-select, drops under the ▾ arrow) — the
   det-pop look, but rows select ONE app instead of toggling a set */
let focPopEl: HTMLElement | null = null;
let focPopDoc = false;

function closeFocusPop() {
  focPopEl?.remove();
  focPopEl = null;
  if (focPopDoc) {
    document.removeEventListener("mousedown", focPopDocHandler);
    focPopDoc = false;
  }
}
function focPopDocHandler(ev: MouseEvent) {
  if (focPopEl && !focPopEl.contains(ev.target as Node)) closeFocusPop();
}

function toggleFocusPop() {
  if (focPopEl) closeFocusPop();
  else openFocusPop();
}

function openFocusPop() {
  closeFocusPop();
  closeWinPop();
  closeDetailsPop();
  const pop = el("div", { class: "focus-pop" });
  pop.append(el("div", { class: "dp-title" }, ["focus app"]));
  const list = el("div", { class: "dp-list" });
  const apps = pickerApps();
  if (!apps.length)
    list.append(el("div", { class: "wp-hint" }, ["no apps yet — they appear once they send requests"]));
  for (const app of apps) {
    const row = el("div", { class: "fp-row" + (app === focusApp ? " sel" : ""), title: esc(app) });
    row.append(el("span", { class: "dp-sw", style: "background:" + lineColor(app) }));
    row.append(el("span", { class: "dp-name" }, [esc(app)]));
    if (app === focusApp) row.append(el("span", { class: "fp-check" }, ["✓"]));
    row.addEventListener("click", () => {
      setFocusApp(app);
      closeFocusPop();
      applyRpsModeUI();
      if (lastMetrics) updateRpsChart(lastMetrics);
    });
    list.append(row);
  }
  pop.append(list);
  pop.append(el("div", { class: "wp-hint" }, ["one app at a time · Focus plots each of its name_id series"]));
  pop.addEventListener("mousedown", (e) => e.stopPropagation());
  ($("#ov-rps-mode") as HTMLElement | null)?.appendChild(pop);
  focPopEl = pop;
  document.addEventListener("mousedown", focPopDocHandler);
  focPopDoc = true;
}

/* apps offered in the picker: live ones plus a saved selection that is
   not currently live (so it stays visible & re-selectable) */
function pickerApps(): string[] {
  const ids = new Set(lastMetrics ? lastMetrics.apps.map((a) => a.app) : []);
  if (focusApp) ids.add(focusApp);
  return [...ids].sort();
}
