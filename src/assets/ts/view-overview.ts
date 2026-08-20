// Overview tab: stat chips, system mini-charts, the all-apps RPS chart
// (shared scale + selectable time window) and the top-apps traffic table.
import { $, el, esc, fmtNum, fmtBytes, fmtUptime } from "./core";
import { sparkline, drawMini, lineColor, getCss } from "./charts";
import { rpsArchive, RPS_WINDOWS, getRpsWindowIdx, setRpsWindowIdx } from "./rps-archive";
import { updateMongoStatus } from "./mongo";
import { lastMetrics, Metrics, AppNode, systemSeries, systemHistory } from "./state";
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
    fmt: (s) => fmtNum(s.net_rx_kbps, 0),
    sub: () => "KB/s in",
  },
  {
    key: "tx",
    label: "Upload",
    color: "--secondary",
    raw: (s) => s.net_tx_kbps,
    fmt: (s) => fmtNum(s.net_tx_kbps, 0),
    sub: () => "KB/s out",
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
        "All apps · RPS",
        el("span", {
          class: "muted",
          id: "ov-rps-summary",
          style: "margin-left:auto;font-weight:400",
        }),
      ]),
      el("div", { id: "ov-rps-legend", class: "rps-legend" }),
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
  const smooth = (key: string, raw: number) => {
    const v = systemSeries[key].push(raw);
    const h = systemHistory[key];
    h.push(v);
    if (h.length > 90) h.shift();
    return h;
  };
  for (const d of chartDefs) {
    const c = chartEls[d.key];
    if (!c) continue;
    c.value.textContent = d.fmt(s);
    c.sub.textContent = d.sub(s);
    drawMini(c.canvas, smooth(d.key, d.raw(s)), getCss(d.color));
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
        : `${active.length} active · ${fmtNum(sumRps, 1)} rps total · worst p50 ${fmtNum(worstP50, 1)}ms · ${fmtNum(lifetime, 0)} requests lifetime`;
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

function updateRpsChart(m: Metrics) {
  const canvas = $("#ov-rps-canvas") as HTMLCanvasElement | null;
  if (!canvas) return;
  const nowMs = Date.now();
  const [, win] = RPS_WINDOWS[getRpsWindowIdx()];
  const apps = m.apps.map((a) => a.app).sort();
  const data = rpsArchive.window(apps, win, Math.floor(nowMs / 1000));
  const series = apps.map((app) => ({ app, color: lineColor(app), pts: data.get(app) ?? [] }));
  let peak = 0;
  for (const s of series) for (const p of s.pts) if (p.v > peak) peak = p.v;
  const legend = $("#ov-rps-legend");
  if (legend) {
    legend.textContent = "";
    const cur = new Map(m.apps.map((a): [string, number] => [a.app, a.rps]));
    for (const app of apps) {
      legend.append(
        el("span", { class: "rl", title: esc(app) }, [
          el("i", { style: "background:" + lineColor(app) }),
          esc(app) + " · " + fmtNum(cur.get(app) ?? 0, 1),
        ]),
      );
    }
  }
  const summary = $("#ov-rps-summary");
  if (summary) {
    const sinceMs = rpsArchive.startSec * 1000;
    const partial = sinceMs > 0 && sinceMs > nowMs - win * 1000;
    summary.textContent =
      `${apps.length} app(s) · shared scale · peak ${fmtNum(peak, 1)} rps` +
      (partial ? ` · collecting since ${fmtAxisTime(sinceMs, win)}` : "");
  }
  const btn = $("#ov-rps-win");
  if (btn) btn.textContent = RPS_WINDOWS[getRpsWindowIdx()][0];
  drawAppRpsChart(canvas, series, win, nowMs);
}

function drawAppRpsChart(
  canvas: HTMLCanvasElement,
  series: { app: string; color: string; pts: { t: number; v: number }[] }[],
  windowSec: number,
  nowMs: number,
) {
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth || 600;
  const h = canvas.clientHeight || 190;
  canvas.width = w * dpr;
  canvas.height = h * dpr;
  const ctx = canvas.getContext("2d")!;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  const ml = 46,
    mr = 10,
    mt = 8,
    mb = 20;
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
  // one line per app, all on the shared scale (x = real time — gaps compress)
  for (const s of series) {
    if (s.pts.length < 1) continue;
    ctx.beginPath();
    let started = false;
    for (const p of s.pts) {
      const x = ml + ((p.t - t0) / windowSec) * iw;
      const y = mt + ih - (Math.min(p.v, vmax) / vmax) * ih;
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
