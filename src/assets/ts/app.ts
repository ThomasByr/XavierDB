// XavierDB dashboard — vanilla TypeScript, zero libraries.
// Hash-routed SPA: overview / clients / config / logs.
// Clients carries the permission editor inline (per app + per name).

/* ============================= helpers ============================= */

const $ = <T extends HTMLElement = HTMLElement>(sel: string): T => document.querySelector(sel) as T;
const $$ = (sel: string): HTMLElement[] => Array.from(document.querySelectorAll(sel));

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Record<string, string> = {},
  children: (Node | string)[] = [],
): HTMLElementTagNameMap[K] {
  const n = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) n.setAttribute(k, v);
  for (const c of children) n.append(c as Node);
  return n;
}

function esc(s: unknown): string {
  return String(s).replace(
    /[&<>"']/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c] as string,
  );
}

function fmtNum(v: number, digits = 1): string {
  if (!isFinite(v)) return "—";
  if (Math.abs(v) >= 1000) return v.toFixed(0);
  return v.toFixed(digits);
}

function fmtBytes(mb: number): string {
  if (mb >= 1024) return (mb / 1024).toFixed(1) + " GB";
  return mb.toFixed(0) + " MB";
}

function timeAgo(ms: number): string {
  if (!ms) return "never";
  const s = Math.max(0, (Date.now() - ms) / 1000);
  if (s < 5) return "now";
  if (s < 60) return fmtNum(s, 0) + "s ago";
  if (s < 3600) return fmtNum(s / 60, 0) + "m ago";
  return fmtNum(s / 3600, 1) + "h ago";
}

function fmtClock(ts: number): string {
  const d = new Date(ts);
  return d.toLocaleTimeString();
}

function fmtUptime(s: number): string {
  if (s < 60) return fmtNum(s, 0) + "s";
  if (s < 3600) return fmtNum(s / 60, 0) + "m";
  if (s < 86400) return fmtNum(s / 3600, 1) + "h";
  return fmtNum(s / 86400, 1) + "d";
}

/* ============================= api ============================= */

let pollSeconds = 2;

async function api(path: string, opts: RequestInit = {}): Promise<any> {
  const res = await fetch("/dashboard/api" + path, {
    headers: { "Content-Type": "application/json" },
    credentials: "same-origin",
    ...opts,
  });
  if (res.status === 401) {
    stopPolling();
    showLogin();
    const b = await res.json().catch(() => ({}));
    throw new Error(b.error || "unauthorized");
  }
  const body = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(body.error || `HTTP ${res.status}`);
  return body;
}

/* ============================= ui primitives ============================= */

let snackTimer = 0;
function snack(msg: string, ms = 2600) {
  const s = $("#snackbar");
  s.textContent = msg;
  s.classList.add("show");
  clearTimeout(snackTimer);
  snackTimer = window.setTimeout(() => s.classList.remove("show"), ms);
}

function confirmDialog(title: string, body: string, okLabel = "Confirm"): Promise<boolean> {
  return new Promise((resolve) => {
    const box = $("#dialog-box");
    box.innerHTML = "";
    box.append(el("h3", {}, [title]));
    box.append(el("p", { class: "muted" }, [body]));
    const actions = el("div", { class: "dialog-actions" });
    const cancel = el("button", { class: "btn btn-text" }, ["Cancel"]);
    cancel.onclick = () => {
      $("#dialog").classList.add("hidden");
      resolve(false);
    };
    const ok = el("button", { class: "btn btn-danger" }, [okLabel]);
    ok.onclick = () => {
      $("#dialog").classList.add("hidden");
      resolve(true);
    };
    actions.append(cancel, ok);
    box.append(actions);
    $("#dialog").classList.remove("hidden");
  });
}

function sparkline(canvas: HTMLCanvasElement, data: number[], color: string, max = 0): void {
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth || 120;
  const h = canvas.clientHeight || 28;
  canvas.width = w * dpr;
  canvas.height = h * dpr;
  const ctx = canvas.getContext("2d")!;
  ctx.scale(dpr, dpr);
  ctx.clearRect(0, 0, w, h);
  if (data.length < 2) return;
  const hi = max > 0 ? max : Math.max(...data, 1e-9);
  const step = w / (data.length - 1);
  ctx.strokeStyle = color;
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  data.forEach((v, i) => {
    const x = i * step;
    const y = h - 2 - (Math.max(0, v) / hi) * (h - 4);
    i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
  });
  ctx.stroke();
}

/* EMA smoothing (client-side) */
class Smoother {
  private buf: number[] = [];
  constructor(
    private window: number,
    private alpha = 0.35,
  ) {}
  push(v: number): number {
    this.buf.push(v);
    if (this.buf.length > this.window) this.buf.shift();
    const base = this.buf[this.buf.length - 1];
    let acc = base;
    for (let i = this.buf.length - 2; i >= 0; i--) acc = this.alpha * this.buf[i] + (1 - this.alpha) * acc;
    return acc;
  }
}

/* ============================= state ============================= */

interface ClientNode {
  name: string;
  id: string;
  blocked: boolean;
  rps: number;
  p50_ms: number;
  total_requests: number;
  last_seen_ms: number;
  rps_history: number[];
}
interface AppNode {
  app: string;
  blocked: boolean;
  weight: number;
  rps: number;
  p50_ms: number;
  limit: number | null;
  rps_history: number[];
  names: ClientNode[];
  breakdown?: any;
}
interface Metrics {
  ts: number;
  config: {
    poll_seconds: number;
    theme: string;
    graph_smoothing: number;
    cfg_version: number;
    perms_version: number;
    health_ttl_seconds: number;
    multiplier: number;
  };
  system: {
    cpu_pct: number;
    mem_pct: number;
    mem_used_mb: number;
    mem_total_mb: number;
    disk_pct: number;
    disk_used_mb: number;
    disk_total_mb: number;
    net_rx_kbps: number;
    net_tx_kbps: number;
    uptime_s: number;
    ts_ms: number;
  };
  qps: number;
  health: any;
  apps: AppNode[];
  cursors: { count: number; list: any[] };
}

let lastMetrics: Metrics | null = null;
let systemSeries: Record<string, Smoother> = {};
let systemHistory: Record<string, number[]> = {
  cpu: [],
  mem: [],
  disk: [],
  rx: [],
  tx: [],
};

function seedSmoothers(m: Metrics) {
  const win = m.config.graph_smoothing || 5;
  if (Object.keys(systemSeries).length === 0) {
    for (const k of Object.keys(systemHistory)) systemSeries[k] = new Smoother(win);
  }
}

/* ============================= rps archive ============================= */

/* Long-window RPS history for the overview "all apps" chart. The server only
   keeps ~120 ticks (~10 min) of sparkline history, so the dashboard samples
   every /metrics poll and downsamples into tiered time buckets, persisted in
   localStorage — windows up to a year survive reloads. Coverage is limited
   to times the dashboard was open (gaps compress, they don't interpolate). */
const RPS_TIERS: [resSec: number, keepSec: number][] = [
  [10, 1800], // 30 min @ 10 s
  [60, 10800], // 3 h @ 1 min
  [300, 43200], // 12 h @ 5 min
  [1800, 259200], // 3 d @ 30 min
  [21600, 1814400], // 21 d @ 6 h
  [86400, 34560000], // 400 d @ 1 d
];
const RPS_WINDOWS: [label: string, sec: number][] = [
  ["1 minute", 60],
  ["3 minutes", 180],
  ["5 minutes", 300],
  ["10 minutes", 600],
  ["20 minutes", 1200],
  ["30 minutes", 1800],
  ["1 hour", 3600],
  ["5 hours", 18000],
  ["1 day", 86400],
  ["3 days", 259200],
  ["1 week", 604800],
  ["2 weeks", 1209600],
  ["1 month", 2629800],
  ["2 months", 5259600],
  ["3 months", 7889400],
  ["1 year", 31557600],
];
const RPS_WINDOW_LS = "xdb-rps-window";
let rpsWindowIdx = (() => {
  const i = parseInt(localStorage.getItem(RPS_WINDOW_LS) ?? "", 10);
  return Number.isInteger(i) && i >= 0 && i < RPS_WINDOWS.length ? i : 3; // 10 minutes
})();

const LINE_PALETTE = [
  "#6d4aff",
  "#00897b",
  "#e53935",
  "#f9a825",
  "#3949ab",
  "#8e24aa",
  "#00acc1",
  "#6d4c41",
  "#43a047",
  "#f4511e",
  "#546e7a",
  "#c2185b",
];
/* stable per-app color (hash of the id — survives re-renders and windows) */
function lineColor(app: string): string {
  let h = 5381;
  for (let i = 0; i < app.length; i++) h = ((h * 33) ^ app.charCodeAt(i)) >>> 0;
  return LINE_PALETTE[h % LINE_PALETTE.length];
}

interface RpsTier {
  ts: number[];
  vs: number[];
  open: { t: number; sum: number; n: number } | null;
}
const RPS_ARCHIVE_LS = "xdb-rps-archive-v1";

class RpsArchive {
  private series: Record<string, { lastT: number; tiers: RpsTier[] }> = {};
  private firstT = 0;
  private lastSaveMs = 0;
  private dirty = false;

  static load(): RpsArchive {
    const a = new RpsArchive();
    try {
      const d = JSON.parse(localStorage.getItem(RPS_ARCHIVE_LS) ?? "null");
      if (d && typeof d.firstT === "number" && d.series && typeof d.series === "object") {
        a.series = d.series;
        a.firstT = d.firstT;
        a.lastSaveMs = Date.now();
      }
    } catch {
      /* corrupted archive — start fresh */
    }
    return a;
  }

  get startSec(): number {
    return this.firstT;
  }

  sample(apps: AppNode[], nowMs: number) {
    const tSec = Math.floor(nowMs / 1000);
    if (!this.firstT) this.firstT = tSec;
    this.dirty = true;
    for (const a of apps) {
      let s = this.series[a.app];
      if (!s) {
        s = { lastT: tSec, tiers: RPS_TIERS.map(() => ({ ts: [], vs: [], open: null })) };
        this.series[a.app] = s;
      }
      s.lastT = tSec;
      const v = Math.max(0, a.rps);
      for (let i = 0; i < RPS_TIERS.length; i++) {
        const [res, keep] = RPS_TIERS[i];
        const tier = s.tiers[i];
        const bt = Math.floor(tSec / res) * res;
        if (tier.open && tier.open.t === bt) {
          tier.open.sum += v;
          tier.open.n++;
        } else {
          if (tier.open) {
            tier.ts.push(tier.open.t);
            tier.vs.push(tier.open.sum / tier.open.n);
          }
          tier.open = { t: bt, sum: v, n: 1 };
          while (tier.ts.length && tier.ts[0] < tSec - keep) {
            tier.ts.shift();
            tier.vs.shift();
          }
        }
      }
    }
    if (Date.now() - this.lastSaveMs > 30000) this.save();
  }

  /* buckets of the finest tier covering `windowSec` (closed + the open one) */
  window(apps: string[], windowSec: number, nowSec: number): Map<string, { t: number; v: number }[]> {
    let ti = RPS_TIERS.findIndex(([, keep]) => keep >= windowSec);
    if (ti < 0) ti = RPS_TIERS.length - 1;
    const res = RPS_TIERS[ti][0];
    const t0 = nowSec - windowSec;
    const out = new Map<string, { t: number; v: number }[]>();
    for (const app of apps) {
      const tier = this.series[app]?.tiers[ti];
      const pts: { t: number; v: number }[] = [];
      if (tier) {
        for (let i = 0; i < tier.ts.length; i++)
          if (tier.ts[i] + res > t0) pts.push({ t: tier.ts[i] + res / 2, v: tier.vs[i] });
        if (tier.open && tier.open.n)
          pts.push({ t: tier.open.t + res / 2, v: tier.open.sum / tier.open.n });
      }
      out.set(app, pts);
    }
    return out;
  }

  flushIfDirty() {
    if (this.dirty) this.save();
  }

  save() {
    this.lastSaveMs = Date.now();
    const nowSec = Math.floor(Date.now() / 1000);
    for (const app of Object.keys(this.series))
      if (nowSec - this.series[app].lastT > 40 * 86400) delete this.series[app]; // dropped apps
    try {
      localStorage.setItem(RPS_ARCHIVE_LS, JSON.stringify({ firstT: this.firstT, series: this.series }));
      this.dirty = false;
    } catch {
      /* quota exceeded — keep running in memory */
    }
  }
}
const rpsArchive = RpsArchive.load();
window.addEventListener("beforeunload", () => {
  rpsArchive.flushIfDirty();
});

/* ============================= login ============================= */

function showLogin() {
  $("#app").classList.add("hidden");
  $("#login-view").classList.remove("hidden");
  $("#login-error").textContent = "";
}

function showApp() {
  $("#login-view").classList.add("hidden");
  $("#app").classList.remove("hidden");
  route();
}

async function checkSession() {
  try {
    await api("/session");
    showApp();
  } catch {
    showLogin();
  }
}

async function initLogin() {
  const doLogin = async () => {
    const user = ($("#login-user") as HTMLInputElement).value.trim();
    const pass = ($("#login-pass") as HTMLInputElement).value;
    if (!user || !pass) return;
    $("#login-btn").setAttribute("disabled", "");
    try {
      await api("/login", {
        method: "POST",
        body: JSON.stringify({ username: user, password: pass }),
      });
      ($("#login-pass") as HTMLInputElement).value = "";
      showApp();
    } catch (e: any) {
      $("#login-error").textContent = e.message;
    } finally {
      $("#login-btn").removeAttribute("disabled");
    }
  };
  $("#login-btn").onclick = doLogin;
  $("#login-pass").addEventListener("keydown", (e) => {
    if (e.key === "Enter") doLogin();
  });
  $("#login-user").addEventListener("keydown", (e) => {
    if (e.key === "Enter") doLogin();
  });
}

/* ============================= router ============================= */

const routes: Record<string, () => void> = {
  overview: renderOverview,
  clients: renderClients,
  config: renderConfig,
  logs: renderLogs,
};

let currentRoute = "overview";
let pollTimer = 0;
let pollEnabled = true;

function route() {
  const hash = location.hash.replace(/^#\//, "").split("?")[0] || "overview";
  currentRoute = routes[hash] ? hash : "overview";
  $$(".nav-item").forEach((n) => n.classList.toggle("active", n.getAttribute("data-route") === currentRoute));
  $("#page-title").textContent = currentRoute[0].toUpperCase() + currentRoute.slice(1);
  routes[currentRoute]();
  // metrics are polled on every tab — views render only on their own route,
  // but the RPS archive (overview chart) must keep sampling everywhere
  pollEnabled = true;
  restartPolling();
}

function stopPolling() {
  clearInterval(pollTimer);
  pollEnabled = false;
}

function restartPolling() {
  clearInterval(pollTimer);
  if (pollEnabled) {
    pollTimer = window.setInterval(poll, Math.max(0.1, pollSeconds) * 1000);
    poll();
  }
}

async function poll() {
  if (!pollEnabled) return;
  try {
    const m: Metrics = await api("/metrics");
    lastMetrics = m;
    pollSeconds = m.config.poll_seconds || 2;
    seedSmoothers(m);
    rpsArchive.sample(m.apps, Date.now());
    if (currentRoute === "overview") renderOverviewData(m);
    if (currentRoute === "clients") renderClientsData(m);
  } catch {
    /* session handled in api() */
  }
}

window.addEventListener("hashchange", route);

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

function renderOverview() {
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
          [RPS_WINDOWS[rpsWindowIdx][0]],
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

function renderOverviewData(m: Metrics) {
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
  if (windowSec <= 2 * 86400)
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (windowSec <= 100 * 86400) return d.toLocaleDateString([], { month: "short", day: "numeric" });
  return d.toLocaleDateString([], { year: "2-digit", month: "short" });
}

function updateRpsChart(m: Metrics) {
  const canvas = $("#ov-rps-canvas") as HTMLCanvasElement | null;
  if (!canvas) return;
  const nowMs = Date.now();
  const [, win] = RPS_WINDOWS[rpsWindowIdx];
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
  if (btn) btn.textContent = RPS_WINDOWS[rpsWindowIdx][0];
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
  const val = el("span", { class: "wp-val" }, [RPS_WINDOWS[rpsWindowIdx][0]]);
  row.append(el("span", { class: "wp-title" }, ["time window"]), val);
  const slider = el("input", {
    type: "range",
    min: "0",
    max: String(RPS_WINDOWS.length - 1),
    step: "1",
    value: String(rpsWindowIdx),
  }) as HTMLInputElement;
  slider.addEventListener("input", () => {
    rpsWindowIdx = parseInt(slider.value, 10);
    val.textContent = RPS_WINDOWS[rpsWindowIdx][0];
    btn.textContent = RPS_WINDOWS[rpsWindowIdx][0];
    localStorage.setItem(RPS_WINDOW_LS, String(rpsWindowIdx));
    if (lastMetrics) updateRpsChart(lastMetrics);
  });
  pop.append(row, slider);
  pop.append(
    el("div", { class: "wp-hint" }, [
      "1 minute → 1 year · history is sampled while the dashboard is open",
    ]),
  );
  pop.addEventListener("mousedown", (e) => e.stopPropagation());
  (btn.parentElement as HTMLElement).appendChild(pop);
  winPopEl = pop;
  document.addEventListener("mousedown", winPopDocHandler);
  winPopDoc = true;
  slider.focus();
}

function getCss(v: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(v).trim() || "#6d4aff";
}

/* single-line mini chart with a soft area fill — no legend, no axis text */
function drawMini(canvas: HTMLCanvasElement, data: number[], color: string) {
  if (!canvas) return;
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth || 200;
  const h = canvas.clientHeight || 48;
  canvas.width = w * dpr;
  canvas.height = h * dpr;
  const ctx = canvas.getContext("2d")!;
  ctx.scale(dpr, dpr);
  ctx.clearRect(0, 0, w, h);
  if (data.length < 2) return;
  const hi = Math.max(...data, 1e-9) * 1.08;
  const step = w / (data.length - 1);
  ctx.beginPath();
  data.forEach((v, i) => {
    const x = i * step;
    const y = h - 2 - (Math.max(0, v) / hi) * (h - 4);
    i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
  });
  ctx.strokeStyle = color;
  ctx.lineWidth = 1.75;
  ctx.lineJoin = "round";
  ctx.stroke();
  ctx.globalAlpha = 0.12;
  ctx.lineTo(w, h);
  ctx.lineTo(0, h);
  ctx.closePath();
  ctx.fillStyle = color;
  ctx.fill();
  ctx.globalAlpha = 1;
}

function updateMongoStatus(h: any) {
  const st = h?.status ?? "unhealthy";
  const m = h?.mongodb ?? {};
  const dot = $("#mongo-dot");
  if (dot) dot.className = "dot " + (st === "ok" ? "ok" : st === "degraded" ? "warn" : "bad");
  const btn = $("#mongo-btn");
  if (!btn) return;
  btn.title =
    st === "ok"
      ? `MongoDB reachable — ping ${fmtNum(m.ping_latency_ms ?? 0, 1)} ms`
      : st === "degraded"
        ? `MongoDB degraded — ping ${fmtNum(m.ping_latency_ms ?? 0, 1)} ms`
        : `MongoDB DOWN — ${m.error ?? "unreachable"}`;
}

async function refreshMongoStatus(): Promise<any> {
  const res = await fetch("/health");
  const h = await res.json().catch(() => ({}));
  if (lastMetrics) lastMetrics.health = h;
  updateMongoStatus(h);
  return h;
}

/* ============================= clients & permissions ============================= */

interface Rule {
  actions: string[];
  databases: string[];
  collections: string[];
  source?: string;
}
interface NamePerm {
  name: string;
  allow: Rule[];
  deny: Rule[];
  effective?: EffectiveRule[];
  delete?: boolean;
}
interface AppPerm {
  app: string;
  token_set: boolean;
  allow: Rule[];
  deny: Rule[];
  effective?: EffectiveRule[];
  names: NamePerm[];
  delete?: boolean;
  set_token?: string;
}
interface EffectiveRule {
  source: string;
  actions: string[];
  databases: string[];
  collections: string[];
}

const ACTIONS = ["GET", "POST", "PUT", "PATCH", "DELETE", "INDEX"];

let clientsSearch = "";
let clientsExpanded: Set<string> = new Set();
let hideCursors = localStorage.getItem("xdb-hide-cursors") !== "0"; // collapsed by default (debugging only)
let permsData: { version: number; apps: AppPerm[] } | null = null;
let dbList: { name: string; collections: string[] }[] = [];
let dbListUnavailable = false;
let permSaveChain: Promise<void> = Promise.resolve();
// scopes seen live but not (yet) in authorized_keys.yml — edited in memory,
// persisted only once they carry content.
const detachApps = new Map<string, AppPerm>();
const detachNames = new Map<string, NamePerm>(); // key: "app\0name"

function renderClients() {
  const v = $("#view");
  v.innerHTML = "";
  v.append(
    el("div", { class: "card" }, [
      el("h3", {}, ["Clients & permissions"]),
      el("div", { class: "toolbar" }, [
        el("input", {
          id: "clients-search",
          type: "text",
          placeholder: "filter by name or app…",
          style:
            "max-width:240px;padding:8px 12px;border:1px solid var(--outline);border-radius:8px;background:var(--surface);color:var(--on-surface);font-size:13px;outline:none",
        }),
        el("span", { class: "muted", id: "clients-summary" }),
        el("span", { class: "spacer" }),
        el("button", { id: "clients-reload", class: "btn btn-outline btn-small" }, ["Reload permissions"]),
        el("button", { id: "clients-add-app", class: "btn btn-outline btn-small" }, ["+ add app"]),
      ]),
      el("div", { id: "clients-tree", class: "tree" }),
    ]),
  );
  v.append(
    el("div", { class: "card" }, [el("h3", {}, ["Adaptive limits"]), el("div", { id: "limits-view" })]),
  );
  const cursorsView = el("div", { id: "cursors-view" });
  const cursorsCard = el("div", { class: "card", id: "cursors-card" }, [
    el("h3", {}, [
      "Live cursors",
      el(
        "button",
        {
          id: "cursors-toggle",
          class: "btn btn-outline btn-small",
          style: "margin-left:auto",
          title: "debugging only",
        },
        [hideCursors ? "Show" : "Hide"],
      ),
    ]),
    cursorsView,
  ]);
  if (hideCursors) cursorsView.classList.add("hidden");
  v.append(cursorsCard);
  $("#cursors-toggle").onclick = () => {
    hideCursors = !hideCursors;
    localStorage.setItem("xdb-hide-cursors", hideCursors ? "1" : "0");
    cursorsView.classList.toggle("hidden", hideCursors);
    ($("#cursors-toggle") as HTMLButtonElement).textContent = hideCursors ? "Show" : "Hide";
  };
  const search = $("#clients-search") as HTMLInputElement;
  search.value = clientsSearch;
  search.oninput = () => {
    clientsSearch = search.value;
    applyClientsFilter();
  };
  $("#clients-reload").onclick = async () => {
    try {
      await api("/perms/reload", { method: "POST" });
      snack("permissions reloaded from disk");
      await loadPermsData();
    } catch (e: any) {
      snack(e.message);
    }
  };
  $("#clients-add-app").onclick = () => addAppDialog();
  loadDatabases();
  loadPermsData();
  if (lastMetrics) renderClientsData(lastMetrics);
}

function applyClientsFilter() {
  const tree = $("#clients-tree");
  if (!tree || !lastMetrics) return;
  const q = clientsSearch.toLowerCase();
  if (!q) {
    for (const n of Array.from(tree.querySelectorAll<HTMLElement>("[data-name]")))
      n.classList.remove("hidden");
    for (const n of Array.from(tree.querySelectorAll<HTMLElement>("[data-app]")))
      n.classList.remove("hidden");
    return;
  }
  for (const node of Array.from(tree.querySelectorAll<HTMLElement>("[data-app]"))) {
    const app = lastMetrics.apps.find((a) => a.app === node.dataset.app);
    if (!app) continue;
    const match =
      !q || app.app.toLowerCase().includes(q) || app.names.some((n) => n.name.toLowerCase().includes(q));
    node.classList.toggle("hidden", !match);
    if (q) {
      const appMatch = app.app.toLowerCase().includes(q);
      for (const nnode of Array.from(node.querySelectorAll<HTMLElement>("[data-name]"))) {
        const name = nnode.dataset.name!.split("@")[0];
        nnode.classList.toggle("hidden", !(name.toLowerCase().includes(q) || appMatch));
      }
    }
  }
}

async function loadDatabases() {
  try {
    const d = await api("/databases");
    dbList = d.databases || [];
    dbListUnavailable = !!d.unavailable;
  } catch {
    dbList = [];
    dbListUnavailable = true;
  }
  rebuildOpenPanels();
}

async function loadPermsData() {
  try {
    permsData = await api("/perms");
  } catch (e: any) {
    snack(e.message);
    return;
  }
  if (!permsData) return;
  // drop detached scopes that have now been persisted
  for (const [k] of detachApps) if (permsData.apps.some((a) => a.app === k)) detachApps.delete(k);
  for (const [k] of detachNames) {
    const [app, name] = k.split("\u0000");
    if (permsData.apps.find((a) => a.app === app)?.names.some((n) => n.name === name)) detachNames.delete(k);
  }
  rebuildOpenPanels();
}

/* merged app list: live apps + file-only apps (so permissions of every
   app_id / name_id are visible and editable even before traffic) */
function mergedApps(m: Metrics) {
  const out = m.apps.map((a) => ({ ...a, names: [...a.names] }));
  const byId = new Map(out.map((a) => [a.app, a]));
  for (const p of permsData?.apps ?? []) {
    let a = byId.get(p.app);
    if (!a) {
      a = {
        app: p.app,
        blocked: false,
        weight: 1,
        rps: 0,
        p50_ms: 0,
        limit: null,
        rps_history: [],
        names: [],
        breakdown: undefined,
      };
      byId.set(p.app, a);
      out.push(a);
    }
    for (const n of p.names) {
      if (!a.names.some((x) => x.name === n.name)) {
        a.names.push({
          name: n.name,
          id: `${n.name}@${p.app}`,
          blocked: false,
          rps: 0,
          p50_ms: 0,
          total_requests: 0,
          last_seen_ms: 0,
          rps_history: [],
        });
      }
    }
    a.names.sort((x, y) => x.name.localeCompare(y.name));
  }
  return out;
}

function renderClientsData(m: Metrics) {
  const tree = $("#clients-tree");
  if (!tree) return;
  updateMongoStatus(m.health);
  const apps = mergedApps(m);
  $("#clients-summary").textContent =
    `${apps.length} app(s), ${apps.reduce((a, b) => a + b.names.length, 0)} name(s)`;
  if (permsData && m.config.perms_version !== permsData.version) loadPermsData();
  const q = clientsSearch.toLowerCase();
  const nodes = new Map<string, HTMLElement>();
  tree.querySelectorAll(".empty-note").forEach((n) => n.remove());
  for (const c of Array.from(tree.children) as HTMLElement[]) if (c.dataset.app) nodes.set(c.dataset.app, c);
  const seen = new Set<string>();
  for (const a of apps) {
    seen.add(a.app);
    let node = nodes.get(a.app);
    if (!node) {
      node = buildAppNode(a);
      tree.append(node);
    }
    updateAppNode(node, a, q);
    updateNames(node, a, q);
  }
  for (const [k, node] of nodes) if (!seen.has(k)) node.remove();
  if (apps.length === 0)
    tree.append(
      el("div", { class: "empty-note" }, [
        "no apps yet — add one with “+ add app”, or edit authorized_keys.yml",
      ]),
    );
  renderLimits(m);
  renderCursors(m);
}

function updateNames(appNode: HTMLElement, a: AppNode, q: string) {
  const ul = appNode.querySelector("ul");
  if (!ul) return;
  const appMatch = !q || a.app.toLowerCase().includes(q);
  const nameNodes = new Map<string, HTMLElement>();
  for (const c of Array.from(ul.children) as HTMLElement[])
    if (c.dataset.name) nameNodes.set(c.dataset.name, c);
  const seenN = new Set<string>();
  for (const n of a.names) {
    seenN.add(n.id);
    let nnode = nameNodes.get(n.id);
    if (!nnode) {
      nnode = buildNameNode(a.app, n);
      ul.append(nnode);
    }
    updateNameNode(nnode, n, q, appMatch);
  }
  for (const [k, c] of nameNodes) if (!seenN.has(k)) c.remove();
}

function buildAppNode(a: AppNode): HTMLElement {
  const node = el("div", { class: "tree-node", "data-app": a.app });
  const row = el("div", { class: "tree-row" });
  const caret = el("span", { class: "caret closed" }, ["▸"]);
  row.append(caret);
  row.append(el("b", {}, [esc(a.app)]));
  row.append(el("span", { class: "badge", "data-role": "blockbadge" }));
  row.append(
    el("span", {
      class: "badge info",
      "data-role": "perm-badge",
      style: "display:none",
      title: "databases granted in authorized_keys.yml",
    }),
  );
  const meta = el("span", { class: "tree-meta" }, [
    el(
      "span",
      {
        class: "weight-label",
        "data-role": "app-weight",
        title: "adaptive-limit weight (×0.1 – ×10) — click to adjust",
      },
      ["×1.0"],
    ),
    el("canvas", {
      class: "sparkline tm-spark",
      "data-role": "app-spark",
      width: "70",
      height: "22",
      style: "width:70px;height:22px",
    }),
    el("span", { class: "tm-rps", "data-role": "app-rps" }),
    el("span", { class: "tm-limit", "data-role": "app-limit" }),
    el("span", { class: "muted tm-p50", "data-role": "app-p50" }),
    el("span", { class: "tm-seen" }), // spacer — aligns name rows' "seen" column
  ]);
  const btn = el("button", {
    class: "btn btn-small btn-outline blockbtn",
    "data-role": "blockbtn",
  });
  btn.onclick = async (e) => {
    e.stopPropagation();
    const cur = lastMetrics?.apps.find((x) => x.app === node.dataset.app);
    const was = cur?.blocked ?? false;
    if (
      !was &&
      !(await confirmDialog(
        `Block app ${node.dataset.app}?`,
        "All names under this app will get 403 BLOCKED.",
        "Block",
      ))
    )
      return;
    try {
      await api(`/${was ? "unblock" : "block"}`, {
        method: "POST",
        body: JSON.stringify({ id: node.dataset.app }),
      });
      snack(`app ${was ? "unblocked" : "blocked"}`);
      poll();
    } catch (err: any) {
      snack(err.message);
    }
  };
  meta.append(btn);
  row.append(meta);

  // weight chip → slider popover
  const wl = meta.querySelector("[data-role=app-weight]") as HTMLElement;
  wl.onclick = (e) => {
    e.stopPropagation();
    const cur = lastMetrics?.apps.find((x) => x.app === a.app)?.weight ?? 1;
    openWeightPop(wl, a.app, cur);
  };

  const expand = el("div", { class: "expand hidden" });
  const panel = el("div", { class: "panel" });
  panel.append(el("div", { class: "panel-title" }, [`permissions · ${esc(a.app)}`]));
  panel.append(el("div", { "data-role": "perm-widget" }));
  panel.append(el("div", { class: "sub-title" }, ["Effective rules"]));
  panel.append(el("div", { "data-role": "perm-eff" }));
  panel.append(
    el("div", {
      "data-role": "adap",
      class: "perms-summary",
      style: "display:none;margin-top:8px",
    }),
  );
  panel.append(el("hr"));
  // token rotation
  const tok = el("div", { class: "row", style: "gap:8px" });
  const tokIn = el("input", {
    type: "password",
    placeholder: "new shared token (min 8 chars)",
    style:
      "flex:1;min-width:160px;max-width:240px;padding:7px 10px;font-size:12px;border:1px solid var(--outline);border-radius:6px;background:var(--surface);color:var(--on-surface);outline:none",
  });
  const tokBtn = el("button", { class: "btn btn-outline btn-small" }, ["Rotate token"]);
  tokBtn.onclick = async () => {
    const t = tokIn.value;
    if (t.length < 8) {
      snack("token too short (min 8 chars)");
      return;
    }
    const entry = appEntry(node.dataset.app!);
    entry.set_token = t;
    tokIn.value = "";
    queuePermsSave();
  };
  tok.append(el("span", { class: "muted" }, ["shared token"]), tokIn, tokBtn);
  panel.append(tok);
  // access check
  const chk = el("div", { class: "row", style: "gap:8px;margin-top:10px" });
  const chkDb = el("input", {
    type: "text",
    placeholder: "database (glob ok)",
    style:
      "width:130px;padding:6px 10px;font-size:12px;border:1px solid var(--outline);border-radius:6px;background:var(--surface);color:var(--on-surface);outline:none",
  });
  const chkColl = el("input", {
    type: "text",
    placeholder: "collection",
    style:
      "width:110px;padding:6px 10px;font-size:12px;border:1px solid var(--outline);border-radius:6px;background:var(--surface);color:var(--on-surface);outline:none",
  });
  const chkBtn = el("button", { class: "btn btn-outline btn-small" }, ["Check access"]);
  const chkOut = el("span", { class: "row", style: "gap:6px" });
  chkBtn.onclick = () => {
    runCheck(chkDb.value || "*", chkColl.value || "*", chkOut, appEntry(node.dataset.app!).effective ?? []);
  };
  chk.append(chkDb, chkColl, chkBtn, chkOut);
  panel.append(chk);
  expand.append(panel);

  const ul = el("ul");
  expand.append(ul);
  node.append(row, expand);
  if (clientsExpanded.has("app:" + a.app)) {
    expand.classList.remove("hidden");
    caret.classList.remove("closed");
    refreshScope(node);
  }

  row.onclick = (e) => {
    e.stopPropagation();
    if ((e.target as HTMLElement).closest("button, input")) return;
    const key = "app:" + a.app;
    const open = !clientsExpanded.has(key);
    open ? clientsExpanded.add(key) : clientsExpanded.delete(key);
    syncExpand(node, key, open);
  };
  return node;
}

function buildNameNode(app: string, n: ClientNode): HTMLElement {
  const node = el("div", { class: "tree-node", "data-name": n.id });
  const row = el("div", { class: "tree-row" });
  const caret = el("span", { class: "caret closed" }, ["▸"]);
  row.append(caret);
  row.append(el("span", {}, [esc(n.name)]));
  row.append(el("span", { class: "badge", "data-role": "blockbadge" }));
  const meta = el("span", { class: "tree-meta" }, [
    el("span", { class: "tm-weight" }), // spacer — aligns app rows' weight chip
    el("canvas", {
      class: "sparkline tm-spark",
      width: "70",
      height: "22",
      style: "width:70px;height:22px",
    }),
    el("span", { class: "tm-rps", "data-role": "name-rps" }),
    el("span", { class: "tm-limit" }), // spacer — aligns app rows' limit column
    el("span", { class: "tm-p50", "data-role": "name-p50" }),
    el("span", { class: "muted tm-seen", "data-role": "name-seen" }),
  ]);
  const btn = el("button", {
    class: "btn btn-small btn-outline blockbtn",
    "data-role": "blockbtn",
  });
  btn.onclick = async (e) => {
    e.stopPropagation();
    const cur = lastMetrics?.apps.find((x) => x.app === app)?.names.find((x) => x.id === n.id);
    const was = cur?.blocked ?? false;
    if (
      !was &&
      !(await confirmDialog(`Block ${n.id}?`, "This name will get 403 BLOCKED on every request.", "Block"))
    )
      return;
    try {
      await api(`/${was ? "unblock" : "block"}`, {
        method: "POST",
        body: JSON.stringify({ id: n.id }),
      });
      snack(`${n.id} ${was ? "unblocked" : "blocked"}`);
      poll();
    } catch (err: any) {
      snack(err.message);
    }
  };
  meta.append(btn);
  row.append(meta);

  const expand = el("div", { class: "expand hidden" });
  const panel = el("div", { class: "panel" });
  panel.append(el("div", { class: "panel-title" }, [`permissions · ${esc(n.name)}`]));
  panel.append(el("div", { "data-role": "perm-widget" }));
  panel.append(el("div", { class: "sub-title" }, ["Effective rules"]));
  panel.append(el("div", { "data-role": "perm-eff" }));
  const del = el("div", { class: "row", style: "margin-top:10px" });
  const delBtn = el("button", { class: "btn btn-outline btn-small" }, ["Delete name"]);
  delBtn.onclick = async (e) => {
    e.stopPropagation();
    if (!(await confirmDialog(`Delete ${n.name}?`, "The name falls back to the app permissions.", "Delete")))
      return;
    const real = permsData?.apps.find((x) => x.app === app)?.names.find((x) => x.name === n.name);
    if (real) {
      real.delete = true;
      queuePermsSave();
    } else snack("no file entry for this name — nothing to delete");
  };
  del.append(delBtn);
  panel.append(del);
  expand.append(panel);
  node.append(row, expand);
  if (clientsExpanded.has("name:" + n.id)) {
    expand.classList.remove("hidden");
    caret.classList.remove("closed");
    refreshScope(node);
  }

  row.onclick = (e) => {
    e.stopPropagation();
    if ((e.target as HTMLElement).closest("button, input")) return;
    const key = "name:" + n.id;
    const open = !clientsExpanded.has(key);
    open ? clientsExpanded.add(key) : clientsExpanded.delete(key);
    syncExpand(node, key, open);
  };
  return node;
}

function syncExpand(node: HTMLElement, key: string, open: boolean) {
  const expand = node.querySelector<HTMLElement>(".expand");
  const caret = node.querySelector<HTMLElement>(".caret");
  if (!expand) return;
  expand.classList.toggle("hidden", !open);
  caret?.classList.toggle("closed", !open);
  if (open) refreshScope(node);
}

function refreshScope(node: HTMLElement) {
  if (node.dataset.app) refreshAppNode(node);
  if (node.dataset.name) refreshNameNode(node);
}

function rebuildOpenPanels() {
  const tree = $("#clients-tree");
  if (!tree) return;
  for (const node of Array.from(tree.querySelectorAll<HTMLElement>("[data-app], [data-name]"))) {
    if (!node.querySelector(".expand")?.classList.contains("hidden")) refreshScope(node);
  }
}

function appEntry(app: string): AppPerm {
  const real = permsData?.apps.find((x) => x.app === app);
  if (real) return real;
  let d = detachApps.get(app);
  if (!d) {
    d = { app, token_set: false, allow: [], deny: [], names: [] };
    detachApps.set(app, d);
  }
  return d;
}

function nameEntry(app: string, name: string): NamePerm {
  const real = permsData?.apps.find((x) => x.app === app)?.names.find((x) => x.name === name);
  if (real) return real;
  const key = app + "\u0000" + name;
  let d = detachNames.get(key);
  if (!d) {
    d = { name, allow: [], deny: [] };
    detachNames.set(key, d);
  }
  return d;
}

function refreshAppNode(node: HTMLElement) {
  const app = node.dataset.app!;
  const widget = node.querySelector<HTMLElement>("[data-role=perm-widget]");
  const eff = node.querySelector<HTMLElement>("[data-role=perm-eff]");
  const badge = node.querySelector<HTMLElement>("[data-role=perm-badge]");
  if (!widget || !permsData) return;
  const entry = appEntry(app);
  const pats = entry.allow.flatMap((r) => r.databases);
  if (badge) {
    if (pats.length) {
      badge.style.display = "";
      badge.textContent = `${pats.length} db${pats.length === 1 ? "" : "s"}`;
    } else badge.style.display = "none";
  }
  renderPermWidget(widget, entry.allow, entry.deny, { scope: "app", eff: entry.effective ?? [] }, () =>
    queuePermsSave(),
  );
  if (eff) renderEffective(eff, entry.effective ?? [], true);
}

function refreshNameNode(node: HTMLElement) {
  const id = node.dataset.name!;
  const [name, app] = id.split("@");
  const widget = node.querySelector<HTMLElement>("[data-role=perm-widget]");
  const eff = node.querySelector<HTMLElement>("[data-role=perm-eff]");
  if (!widget || !permsData || !app) return;
  const entry = nameEntry(app, name);
  renderPermWidget(widget, entry.allow, entry.deny, { scope: "name", eff: entry.effective ?? [] }, () =>
    queuePermsSave(),
  );
  if (eff) renderEffective(eff, entry.effective ?? [], false);
}

function updateAppNode(node: HTMLElement, a: AppNode, q: string) {
  const rps = node.querySelector("[data-role=app-rps]");
  const lim = node.querySelector("[data-role=app-limit]");
  const p50 = node.querySelector("[data-role=app-p50]");
  const bb = node.querySelector("[data-role=blockbadge]");
  const btn = node.querySelector("[data-role=blockbtn]");
  if (rps) rps.textContent = `${fmtNum(a.rps, 1)} rps`;
  if (lim) lim.textContent = `limit ${a.limit ?? "—"}`;
  if (p50) p50.textContent = `p50 ${fmtNum(a.p50_ms, 1)}ms`;
  const wl = node.querySelector("[data-role=app-weight]") as HTMLElement | null;
  if (wl) {
    const w = a.weight ?? 1;
    wl.textContent = "×" + w.toFixed(1);
    wl.classList.toggle("w-alt", w !== 1);
  }
  const asp = node.querySelector("[data-role=app-spark]") as HTMLCanvasElement;
  if (asp) sparkline(asp, a.rps_history, getCss("--primary"));
  if (bb) {
    bb.className = "badge " + (a.blocked ? "bad" : "ok");
    bb.textContent = a.blocked ? "BLOCKED" : "active";
  }
  if (btn) btn.textContent = a.blocked ? "unblock" : "block";
  const badge = node.querySelector("[data-role=perm-badge]") as HTMLElement | null;
  if (badge && permsData) {
    const p = permsData.apps.find((x) => x.app === a.app) ?? detachApps.get(a.app);
    const pats = p?.allow.flatMap((r) => r.databases) ?? [];
    if (pats.length) {
      badge.style.display = "";
      badge.textContent = `${pats.length} db${pats.length === 1 ? "" : "s"}`;
    } else badge.style.display = "none";
  }
  const adap = node.querySelector("[data-role=adap]") as HTMLElement | null;
  if (adap) {
    const bd = a.breakdown;
    if (bd) {
      adap.style.display = "";
      adap.textContent = `adaptive: p50 ${fmtNum(bd.p50_ms, 1)}ms · shrink ${fmtNum(bd.shrink, 2)} · pressure ${fmtNum(bd.pressure, 2)} · lat_err ${fmtNum(bd.lat_err, 2)} · rate ${fmtNum(bd.rate, 1)}/s`;
    } else adap.style.display = "none";
  }
  node.classList.toggle(
    "hidden",
    !!q && !a.app.toLowerCase().includes(q) && !a.names.some((n) => n.name.toLowerCase().includes(q)),
  );
}

function updateNameNode(node: HTMLElement, n: ClientNode, q: string, appMatch: boolean) {
  const rps = node.querySelector("[data-role=name-rps]");
  const p50 = node.querySelector("[data-role=name-p50]");
  const seen = node.querySelector("[data-role=name-seen]");
  const bb = node.querySelector("[data-role=blockbadge]");
  const btn = node.querySelector("[data-role=blockbtn]");
  if (rps) rps.textContent = `${fmtNum(n.rps, 1)} rps`;
  if (p50) p50.textContent = `p50 ${fmtNum(n.p50_ms, 1)}ms`;
  if (seen) seen.textContent = timeAgo(n.last_seen_ms);
  if (bb) {
    bb.className = "badge " + (n.blocked ? "bad" : "ok");
    bb.textContent = n.blocked ? "BLOCKED" : "active";
  }
  if (btn) btn.textContent = n.blocked ? "unblock" : "block";
  const cv = node.querySelector(".sparkline") as HTMLCanvasElement;
  if (cv) sparkline(cv, n.rps_history, getCss("--primary"));
  node.classList.toggle("hidden", !!q && !n.name.toLowerCase().includes(q) && !appMatch);
}

/* weight chip → slider popover */
let weightPopEl: HTMLElement | null = null;
let weightPopDoc = false;

function closeWeightPop() {
  weightPopEl?.remove();
  weightPopEl = null;
  if (weightPopDoc) {
    document.removeEventListener("mousedown", weightPopDocHandler);
    weightPopDoc = false;
  }
}
function weightPopDocHandler(ev: MouseEvent) {
  if (weightPopEl && !weightPopEl.contains(ev.target as Node)) closeWeightPop();
}

function openWeightPop(anchor: HTMLElement, app: string, cur: number) {
  closeWeightPop();
  const pop = el("div", { class: "weight-pop" });
  const row = el("div", { class: "wp-row" });
  const val = el("span", { class: "wp-val" }, ["×" + cur.toFixed(1)]);
  row.append(el("span", { class: "wp-title" }, ["limit weight · " + esc(app)]), val);
  const slider = el("input", {
    type: "range",
    min: "0.1",
    max: "10",
    step: "0.1",
    value: String(cur),
  }) as HTMLInputElement;
  slider.addEventListener("input", () => {
    val.textContent = "×" + parseFloat(slider.value).toFixed(1);
  });
  slider.addEventListener("change", async () => {
    const w = parseFloat(slider.value);
    try {
      await api("/app_weight", {
        method: "POST",
        body: JSON.stringify({ id: app, weight: w }),
      });
      anchor.textContent = "×" + w.toFixed(1);
      anchor.classList.toggle("w-alt", w !== 1);
      snack(`${app} weight → ×${w.toFixed(1)}`);
    } catch (e: any) {
      snack(e.message);
    }
  });
  pop.append(row, slider);
  pop.append(el("div", { class: "wp-hint" }, ["enforced = limit × multiplier × weight · 0.1× – 10×"]));
  pop.addEventListener("mousedown", (e) => e.stopPropagation());
  anchor.offsetParent!.appendChild(pop);
  weightPopEl = pop;
  document.addEventListener("mousedown", weightPopDocHandler);
  weightPopDoc = true;
  slider.focus();
}

/* ---------- the permission widget ---------- */

type ActState = "a" | "d"; // explicit allow / deny; absence = inherit / no rule

interface PermEntry {
  pattern: string;
  isGlob: boolean;
  db: Record<string, ActState>; // action -> db-level state
  colls: Map<string, Record<string, ActState>>; // collection (or glob) -> action states
}

interface PermCtx {
  scope: "app" | "name";
  eff: EffectiveRule[]; // layered rules of this scope, in resolution order
}

function entriesFromRules(allow: Rule[], deny: Rule[]): PermEntry[] {
  const map = new Map<string, PermEntry>();
  const add = (r: Rule, st: ActState) => {
    for (const db of r.databases) {
      let e = map.get(db);
      if (!e) {
        e = { pattern: db, isGlob: /[*?]/.test(db), db: {}, colls: new Map() };
        map.set(db, e);
      }
      if (r.collections.includes("*")) {
        for (const a of r.actions) e.db[a] = st;
      } else {
        for (const c of r.collections) {
          const cs = e.colls.get(c) ?? {};
          for (const a of r.actions) cs[a] = st;
          e.colls.set(c, cs);
        }
      }
    }
  };
  for (const r of allow) add(r, "a");
  for (const r of deny) add(r, "d");
  return [...map.values()];
}

function applyEntries(allow: Rule[], deny: Rule[], entries: PermEntry[]) {
  allow.length = 0;
  deny.length = 0;
  for (const e of entries) {
    const dbA: string[] = [],
      dbD: string[] = [];
    for (const a of ACTIONS) {
      if (e.db[a] === "a") dbA.push(a);
      else if (e.db[a] === "d") dbD.push(a);
    }
    if (dbA.length) allow.push({ actions: dbA, databases: [e.pattern], collections: ["*"] });
    if (dbD.length) deny.push({ actions: dbD, databases: [e.pattern], collections: ["*"] });
    for (const [c, cs] of e.colls) {
      const ca: string[] = [],
        cd: string[] = [];
      for (const a of ACTIONS) {
        if (cs[a] === "a") ca.push(a);
        else if (cs[a] === "d") cd.push(a);
      }
      if (ca.length) allow.push({ actions: ca, databases: [e.pattern], collections: [c] });
      if (cd.length) deny.push({ actions: cd, databases: [e.pattern], collections: [c] });
    }
  }
}

function glob(pattern: string, value: string): boolean {
  if (pattern === "*" || pattern === "**") return true;
  const re = new RegExp(
    "^" +
      pattern
        .replace(/[.+^${}()|[\]\\]/g, "\\$&")
        .replace(/\*/g, ".*")
        .replace(/\?/g, ".") +
      "$",
  );
  return re.test(value);
}

/* effective verdict for (action, db pattern, collection) — first match wins */
function effVerdict(
  eff: EffectiveRule[],
  action: string,
  db: string,
  coll: string,
  appOnly: boolean,
): "allow" | "deny" | null {
  for (const r of eff) {
    if (appOnly && r.source.startsWith("name_")) continue;
    if (!r.actions.includes(action)) continue;
    if (!r.databases.some((p) => glob(p, db))) continue;
    if (!r.collections.some((p) => glob(p, coll))) continue;
    return r.source.endsWith("deny") ? "deny" : "allow";
  }
  return null;
}

interface WidgetUIState {
  expanded: Set<string>;
  search: string;
}
function widgetUI(cont: HTMLElement): WidgetUIState {
  const s = (cont as any)._wui as WidgetUIState | undefined;
  if (s) return s;
  const n: WidgetUIState = { expanded: new Set(), search: "" };
  (cont as any)._wui = n;
  return n;
}

function renderPermWidget(cont: HTMLElement, allow: Rule[], deny: Rule[], ctx: PermCtx, onSave: () => void) {
  cont.innerHTML = "";
  const wui = widgetUI(cont);
  const entries = entriesFromRules(allow, deny);
  const activeGlobs = entries.filter((e) => e.isGlob && (Object.keys(e.db).length > 0 || e.colls.size > 0));
  const lockedBy = (name: string) => activeGlobs.find((g) => glob(g.pattern, name))?.pattern ?? "";
  const globVerdict = (action: string): ActState | null => {
    let a = false,
      d = false;
    for (const g of activeGlobs) {
      if (g.db[action] === "a") a = true;
      if (g.db[action] === "d") d = true;
    }
    return d ? "d" : a ? "a" : null;
  };
  const hasWild = (s: string) => s.includes("*") || s.includes("?");
  const matches = (q: string, name: string) =>
    hasWild(q) ? glob(q, name) : name.toLowerCase().includes(q.toLowerCase());
  const commit = () => {
    applyEntries(allow, deny, entries);
    onSave();
  };
  const next = (s: ActState | null): ActState | null => (s === "a" ? "d" : s === "d" ? null : "a");

  // legend with real badge samples
  const legend = el("div", { class: "perm-legend" });
  const sample = (cls: string, label: string) => {
    const s = el("span", { class: "act " + cls }, ["GET"]);
    s.setAttribute("disabled", "");
    legend.append(s, el("span", { class: "llabel" }, [label]));
  };
  sample("allow", "allow");
  sample("deny", "deny");
  if (ctx.scope === "name") {
    sample("inherit-allow", "inherits app");
    sample("inherit-deny", "inherits app (denied)");
  } else {
    legend.append(el("span", { class: "llabel" }, ["gray in collections = inherits the database ·"]));
  }
  sample("none", "no rule");
  cont.append(legend);

  // search + add-pattern toggle
  const searchRow = el("div", { class: "perm-search" });
  const input = el("input", {
    type: "text",
    placeholder: "search… or add a database / glob pattern (e.g. analytics_*)",
  });
  const addLabel = el("label", {
    class: "switch",
    title: "add the pattern as a permission entry (all actions allowed)",
  });
  const addCb = el("input", { type: "checkbox" }) as HTMLInputElement;
  addLabel.append(addCb, el("span", { class: "track" }), el("span", { class: "thumb" }));
  addCb.disabled = !wui.search;
  input.value = wui.search;
  searchRow.append(input, addLabel);
  cont.append(searchRow);

  const dbsBox = el("div");
  const globsBox = el("div");
  cont.append(dbsBox, globsBox);

  const actBadge = (
    action: string,
    cls: string,
    title: string,
    onClick: (() => void) | null,
  ): HTMLElement => {
    const b = el("span", { class: "act " + cls, title }, [action]);
    if (onClick) b.onclick = onClick;
    else b.setAttribute("disabled", "");
    return b;
  };

  /* display state of one action badge */
  const dbDisp = (
    action: string,
    own: ActState | null,
    lock: string,
    pat: string,
  ): { cls: string; title: string } => {
    if (lock) {
      const gv = globVerdict(action);
      const deny = own === "d" || gv === "d";
      const allow = !deny && (own === "a" || gv === "a");
      return deny
        ? { cls: "inherit-deny", title: `denied by pattern ${lock}` }
        : allow
          ? { cls: "inherit-allow", title: `allowed by pattern ${lock}` }
          : { cls: "none", title: `not covered by pattern ${lock}` };
    }
    if (own === "a") return { cls: "allow", title: "allowed (explicit) — click to deny" };
    if (own === "d") return { cls: "deny", title: "denied (explicit) — click to clear" };
    if (ctx.scope === "name") {
      const v = effVerdict(ctx.eff, action, pat, "*", true);
      if (v === "allow")
        return {
          cls: "inherit-allow",
          title: "inherits app — allowed — click to override",
        };
      if (v === "deny")
        return {
          cls: "inherit-deny",
          title: "inherits app — denied — click to override",
        };
    }
    return {
      cls: "none",
      title: "no rule — denied by default — click to allow",
    };
  };
  const collDisp = (
    action: string,
    own: ActState | null,
    lock: string,
    pat: string,
    coll: string,
  ): { cls: string; title: string } => {
    if (own === "a") return { cls: "allow", title: "override: allowed — click to deny" };
    if (own === "d") return { cls: "deny", title: "override: denied — click to clear" };
    const v = lock ? globVerdict(action) : effVerdict(ctx.eff, action, pat, coll, false);
    if (lock) {
      return v === "d"
        ? { cls: "inherit-deny", title: `denied by pattern ${lock}` }
        : v === "a"
          ? { cls: "inherit-allow", title: `allowed by pattern ${lock}` }
          : { cls: "none", title: `not covered by pattern ${lock}` };
    }
    const base =
      v === "allow" ? "database allows it" : v === "deny" ? "database denies it" : "nothing grants it";
    return {
      cls: "inherit",
      title: `inherits database — ${base} — click to override`,
    };
  };

  /* the 6 action badges of one row */
  const rowBadges = (
    pat: string,
    entry: PermEntry | null | undefined,
    lock: string,
    dbLevel: boolean,
    coll?: string,
  ): HTMLElement => {
    const acts = el("span", { class: "acts" });
    for (const action of ACTIONS) {
      const own = entry
        ? dbLevel
          ? (entry.db[action] ?? null)
          : (entry.colls.get(coll!)?.[action] ?? null)
        : null;
      const disp = dbLevel ? dbDisp(action, own, lock, pat) : collDisp(action, own, lock, pat, coll!);
      const onClick = lock
        ? null
        : () => {
            let e = entry;
            if (!e) {
              e = { pattern: pat, isGlob: false, db: {}, colls: new Map() };
              entries.push(e);
            }
            if (dbLevel) {
              const nv = next(e.db[action] ?? null);
              if (nv === null) delete e.db[action];
              else e.db[action] = nv;
            } else {
              let cs = e.colls.get(coll!);
              if (!cs) {
                cs = {};
                e.colls.set(coll!, cs);
              }
              const nv = next(cs[action] ?? null);
              if (nv === null) delete cs[action];
              else cs[action] = nv;
            }
            commit();
            renderRows();
          };
      acts.append(actBadge(action, disp.cls, disp.title, onClick));
    }
    return acts;
  };

  const renderColls = (entry: PermEntry | null | undefined, pat: string, lock: string): HTMLElement => {
    const box = el("div", { class: "perm-colls" });
    const realColls = dbList.find((d) => d.name === pat)?.collections ?? [];
    const rows: [string, boolean][] = [
      ...realColls.map((c): [string, boolean] => [c, true]),
      ...(entry
        ? [...entry.colls.keys()]
            .filter((c) => !realColls.includes(c))
            .map((c): [string, boolean] => [c, false])
        : []),
    ];
    for (const [coll, real] of rows) {
      const r = el("div", { class: "perm-row" + (real ? "" : " dim") });
      r.append(
        el(
          "span",
          {
            class: "perm-name",
            title: real ? "" : "collection pattern from the permission file",
          },
          [esc(coll)],
        ),
      );
      r.append(rowBadges(pat, entry, lock, false, coll));
      if (entry?.colls.has(coll)) {
        const reset = el("button", { class: "mini-btn", title: "remove this override" }, ["↺"]);
        reset.onclick = (ev) => {
          ev.stopPropagation();
          entry.colls.delete(coll);
          commit();
          renderRows();
        };
        r.append(reset);
      }
      box.append(r);
    }
    if (!lock) {
      const add = el("div", { class: "perm-add" });
      const ai = el("input", {
        type: "text",
        placeholder: "collection or glob (e.g. log_*)",
      });
      const ab = el(
        "button",
        {
          class: "mini-btn add",
          title: "add a collection override (all actions allowed)",
        },
        ["+ add"],
      );
      const doAdd = () => {
        const c = ai.value.trim();
        if (!c) return;
        let e = entry;
        if (!e) {
          e = { pattern: pat, isGlob: false, db: {}, colls: new Map() };
          entries.push(e);
        }
        if (e.colls.has(c)) {
          snack(`override for “${c}” already exists`);
          return;
        }
        const cs: Record<string, ActState> = {};
        for (const a of ACTIONS) cs[a] = "a";
        e.colls.set(c, cs);
        ai.value = "";
        commit();
        renderRows();
        snack(`override added for “${c}”`);
      };
      ab.onclick = (ev) => {
        ev.stopPropagation();
        doAdd();
      };
      ai.addEventListener("keydown", (ev) => {
        if (ev.key === "Enter") {
          ev.stopPropagation();
          doAdd();
        }
      });
      add.append(ai, ab);
      box.append(add);
    }
    return box;
  };

  const renderRows = () => {
    dbsBox.innerHTML = "";
    globsBox.innerHTML = "";
    wui.search = input.value;
    const q = input.value.trim();
    const entryFor = (pat: string) => entries.find((e) => e.pattern === pat);
    const realDbs = [...dbList].sort((a, b) => a.name.localeCompare(b.name));
    const extras = entries
      .filter((e) => !e.isGlob && !dbList.some((d) => d.name === e.pattern))
      .map((e) => e.pattern);

    for (const [pat, real] of [
      ...realDbs.map((d): [string, boolean] => [d.name, true]),
      ...extras.map((p): [string, boolean] => [p, false]),
    ]) {
      if (q && !matches(q, pat)) continue;
      const entry = entryFor(pat);
      const lock = lockedBy(pat);
      const row = el("div", { class: "perm-row" + (real ? "" : " dim") });
      const caret = el(
        "span",
        {
          class: "caret" + (wui.expanded.has(pat) ? "" : " closed"),
          title: "collections",
        },
        ["▸"],
      );
      caret.onclick = (ev) => {
        ev.stopPropagation();
        wui.expanded.has(pat) ? wui.expanded.delete(pat) : wui.expanded.add(pat);
        renderRows();
      };
      row.append(caret);
      row.append(
        el(
          "span",
          {
            class: "perm-name",
            title: real ? "" : "not in MongoDB — pattern from the permission file",
          },
          [esc(pat)],
        ),
      );
      row.append(rowBadges(pat, entry, lock, true));
      if (lock)
        row.append(
          el(
            "span",
            {
              class: "perm-lock",
              title: `controlled by pattern ${lock} — edit or delete it to change this database`,
            },
            ["🔒"],
          ),
        );
      dbsBox.append(row);
      if (wui.expanded.has(pat)) dbsBox.append(renderColls(entry, pat, lock));
    }
    if (dbsBox.children.length === 0) {
      dbsBox.append(
        el("div", { class: "muted", style: "padding:4px 6px" }, [
          q
            ? `no database matches “${esc(q)}”`
            : "no databases yet — type a pattern above and toggle it to grant access",
        ]),
      );
    } else if (dbListUnavailable) {
      dbsBox.append(
        el("div", { class: "muted", style: "padding:4px 6px" }, [
          "MongoDB unreachable — listing permission-file patterns only",
        ]),
      );
    }

    const globs = entries.filter((e) => e.isGlob);
    if (globs.length) {
      globsBox.append(el("div", { class: "sub-title" }, [`Glob patterns (${globs.length})`]));
      for (const e of globs) {
        const row = el("div", { class: "perm-row" });
        const caret = el(
          "span",
          {
            class: "caret" + (wui.expanded.has(e.pattern) ? "" : " closed"),
            title: "collections",
          },
          ["▸"],
        );
        caret.onclick = (ev) => {
          ev.stopPropagation();
          wui.expanded.has(e.pattern) ? wui.expanded.delete(e.pattern) : wui.expanded.add(e.pattern);
          renderRows();
        };
        row.append(caret);
        row.append(el("span", { class: "perm-name" }, [esc(e.pattern)]));
        row.append(rowBadges(e.pattern, e, "", true));
        const n = dbList.filter((d) => glob(e.pattern, d.name)).length;
        if (n) row.append(el("span", { class: "perm-hint" }, [`${n} db${n === 1 ? "" : "s"}`]));
        const reset = el("button", { class: "mini-btn", title: "deactivate (clear all actions)" }, ["↺"]);
        reset.onclick = (ev) => {
          ev.stopPropagation();
          e.db = {};
          e.colls.clear();
          commit();
          renderRows();
        };
        const del = el("button", { class: "mini-btn", title: "delete pattern" }, ["✕"]);
        del.onclick = (ev) => {
          ev.stopPropagation();
          entries.splice(entries.indexOf(e), 1);
          commit();
          renderRows();
        };
        row.append(el("span", { class: "perm-tools" }, [reset, del]));
        globsBox.append(row);
        if (wui.expanded.has(e.pattern)) globsBox.append(renderColls(e, e.pattern, ""));
      }
    }
  };

  input.oninput = () => {
    wui.search = input.value;
    addCb.disabled = !input.value.trim();
    renderRows();
  };
  addCb.onchange = () => {
    if (!addCb.checked) return;
    addCb.checked = false;
    const pattern = input.value.trim();
    if (!pattern) return;
    let e = entries.find((x) => x.pattern === pattern);
    if (e) {
      for (const a of ACTIONS) e.db[a] = "a";
    } else {
      const db: Record<string, ActState> = {};
      for (const a of ACTIONS) db[a] = "a";
      entries.push({ pattern, isGlob: hasWild(pattern), db, colls: new Map() });
    }
    input.value = "";
    wui.search = "";
    addCb.disabled = true;
    commit();
    renderRows();
    snack(`granted ${hasWild(pattern) ? "pattern" : "database"} “${pattern}”`);
  };
  renderRows();
}

function renderEffective(cont: HTMLElement, eff: EffectiveRule[], appScope: boolean) {
  cont.innerHTML = "";
  if (!eff.length) {
    cont.append(
      el("div", { class: "muted" }, [
        appScope
          ? "no rules — every request is denied by default"
          : "no own rules — inherits the app permissions",
      ]),
    );
    return;
  }
  const t = el("table", { class: "data table-sm" });
  t.innerHTML =
    "<thead><tr><th>source</th><th>actions</th><th>databases</th><th>collections</th></tr></thead>";
  const tb = el("tbody");
  for (const r of eff) {
    const tr = el("tr");
    tr.append(
      el("td", {}, [
        el("span", { class: "badge " + (r.source.endsWith("deny") ? "bad" : "info") }, [r.source]),
      ]),
    );
    tr.append(el("td", {}, [r.actions.join(", ")]));
    tr.append(el("td", {}, [r.databases.join(", ")]));
    tr.append(el("td", {}, [r.collections.join(", ")]));
    tb.append(tr);
  }
  t.append(tb);
  cont.append(t);
}

function runCheck(db: string, coll: string, out: HTMLElement, eff: EffectiveRule[]) {
  out.innerHTML = "";
  for (const action of ACTIONS) {
    let verdict = false;
    for (const r of eff) {
      if (
        r.actions.includes(action) &&
        r.databases.some((p) => glob(p, db)) &&
        r.collections.some((p) => glob(p, coll))
      ) {
        verdict = !r.source.endsWith("deny");
        break;
      }
    }
    out.append(
      el("span", { class: "badge " + (verdict ? "ok" : "bad") }, [`${action} ${verdict ? "✓" : "✗"}`]),
    );
  }
}

function renderLimits(m: Metrics) {
  const cv = $("#limits-view");
  if (!cv) return;
  cv.innerHTML = "";
  const withBd = m.apps.filter((a) => a.breakdown);
  if (!withBd.length) {
    cv.append(
      el("div", { class: "muted" }, ["no adaptive limit state yet — appears once apps send traffic"]),
    );
    return;
  }
  const t = el("table", { class: "data table-sm" });
  t.innerHTML =
    "<thead><tr><th>app</th><th>limit</th><th>enforced</th><th>rate</th><th>p50</th><th>lat_err</th><th>pressure</th><th>shrink</th></tr></thead>";
  const tb = el("tbody");
  for (const a of withBd) {
    const bd = a.breakdown;
    const tr = el("tr");
    tr.append(el("td", {}, [esc(a.app)]));
    tr.append(el("td", { class: "tnum" }, [String(a.limit ?? "—")]));
    tr.append(el("td", { class: "tnum" }, [String(bd.enforced)]));
    tr.append(el("td", { class: "tnum" }, [fmtNum(bd.rate, 1)]));
    tr.append(el("td", { class: "tnum" }, [fmtNum(bd.p50_ms, 1)]));
    tr.append(el("td", { class: "tnum" }, [fmtNum(bd.lat_err, 2)]));
    tr.append(el("td", { class: "tnum" }, [fmtNum(bd.pressure, 2)]));
    tr.append(el("td", { class: "tnum" }, [fmtNum(bd.shrink, 3)]));
    tb.append(tr);
  }
  t.append(tb);
  cv.append(t);
}

function renderCursors(m: Metrics) {
  const cv = $("#cursors-view");
  if (!cv) return;
  cv.innerHTML = "";
  cv.append(el("div", { class: "muted" }, [`${m.cursors.count} active`]));
  if (m.cursors.list.length === 0) {
    cv.append(el("div", { class: "empty-note" }, ["no cursors — paginated requests create them"]));
    return;
  }
  const t = el("table", { class: "data table-sm" });
  t.innerHTML =
    "<thead><tr><th>id</th><th>collection</th><th>created</th><th>last used</th><th>pages</th></tr></thead>";
  const tb = el("tbody");
  for (const c of m.cursors.list) {
    const tr = el("tr");
    tr.append(el("td", { class: "mono" }, [c.id]));
    tr.append(el("td", {}, [`${esc(c.db)}.${esc(c.coll)}`]));
    tr.append(el("td", {}, [timeAgo(c.created_ms)]));
    tr.append(el("td", {}, [timeAgo(c.last_used_ms)]));
    tr.append(el("td", {}, [String(c.uses)]));
    tb.append(tr);
  }
  t.append(tb);
  cv.append(t);
}

/* ---------- saving ---------- */

function queuePermsSave() {
  permSaveChain = permSaveChain.then(savePerms).catch((e) => snack("permissions: " + (e.message || e)));
}

async function savePerms() {
  if (!permsData) return;
  type NamePayload = {
    name: string;
    allow?: Rule[];
    deny?: Rule[];
    delete?: boolean;
  };
  type AppPayload = {
    app: string;
    allow: Rule[];
    deny: Rule[];
    names: NamePayload[];
    delete?: boolean;
    set_token?: string;
  };
  const apps: AppPayload[] = permsData.apps.map((a) => ({
    app: a.app,
    allow: a.allow,
    deny: a.deny,
    names: a.names.map((n) =>
      n.delete ? { name: n.name, delete: true } : { name: n.name, allow: n.allow, deny: n.deny },
    ),
    delete: a.delete,
    set_token: a.set_token,
  }));
  // detached apps: persist only once they carry content (avoids creating
  // empty file entries for live-only identities)
  for (const [app, d] of detachApps) {
    if (d.allow.length || d.deny.length || d.set_token) {
      apps.push({
        app,
        allow: d.allow,
        deny: d.deny,
        names: detachedNamesFor(app).map((n) => ({
          name: n.name,
          allow: n.allow,
          deny: n.deny,
        })),
        set_token: d.set_token,
      });
    }
  }
  // detached names under real apps
  for (const a of permsData.apps) {
    const extra = detachedNamesFor(a.app).filter((n) => !a.names.some((x) => x.name === n.name));
    if (extra.length) {
      apps
        .find((x) => x.app === a.app)!
        .names.push(...extra.map((n) => ({ name: n.name, allow: n.allow, deny: n.deny })));
    }
  }
  await api("/perms", { method: "POST", body: JSON.stringify({ apps }) });
  await loadPermsData();
}

function detachedNamesFor(app: string): NamePerm[] {
  return [...detachNames.entries()]
    .filter(([k]) => k.startsWith(app + "\u0000"))
    .filter(([, n]) => n.allow.length || n.deny.length)
    .map(([, n]) => n);
}

function addAppDialog() {
  const box = $("#dialog-box");
  box.innerHTML = "";
  box.append(el("h3", {}, ["Add app"]));
  box.append(
    el("div", { class: "field" }, [
      el("label", {}, ["app_id"]),
      el("input", {
        id: "new-app-id",
        type: "text",
        placeholder: "e.g. provider2",
        spellcheck: "false",
      }),
    ]),
  );
  box.append(
    el("div", { class: "field" }, [
      el("label", {}, ["shared token (min 8 chars)"]),
      el("input", { id: "new-app-token", type: "password" }),
    ]),
  );
  const actions = el("div", { class: "dialog-actions" });
  const cancel = el("button", { class: "btn btn-text" }, ["Cancel"]);
  cancel.onclick = () => $("#dialog").classList.add("hidden");
  const ok = el("button", { class: "btn" }, ["Create"]);
  ok.onclick = async () => {
    const id = ($("#new-app-id") as HTMLInputElement).value.trim();
    const tok = ($("#new-app-token") as HTMLInputElement).value;
    if (!id || tok.length < 8) {
      snack("app_id and a token of min 8 chars are required");
      return;
    }
    if (!permsData) permsData = { version: 0, apps: [] };
    permsData.apps.push({
      app: id,
      token_set: false,
      allow: [],
      deny: [],
      names: [],
      set_token: tok,
    });
    try {
      await savePerms();
      $("#dialog").classList.add("hidden");
      clientsExpanded.add("app:" + id);
      snack("app created — grant databases below");
      if (lastMetrics) renderClientsData(lastMetrics);
      else rebuildOpenPanels();
    } catch (e: any) {
      snack(e.message);
    }
  };
  actions.append(cancel, ok);
  box.append(actions);
  $("#dialog").classList.remove("hidden");
  $("#new-app-id").focus();
}

/* ============================= config ============================= */

let configData: any = null;
let configDirty = false;
let configSaving = false;

function markCfgDirty() {
  if (configDirty) return;
  configDirty = true;
  updateCfgSave();
}
function updateCfgSave() {
  const btn = $("#cfg-save") as HTMLButtonElement | null;
  if (btn) btn.disabled = configSaving || !configDirty;
  const d = $("#cfg-dirty");
  if (d) d.classList.toggle("hidden", !configDirty);
}

function renderConfig() {
  const v = $("#view");
  v.innerHTML = "";
  v.append(
    el("div", { class: "card" }, [
      el("h3", {}, [
        "Configuration (binary file `config` — settings, adaptive rate limiting, blocked list)",
        el(
          "span",
          {
            id: "cfg-dirty",
            class: "badge warn hidden",
            style: "margin-left:auto",
          },
          ["unsaved changes"],
        ),
      ]),
      el("div", { class: "row", style: "margin-bottom:12px" }, [
        el("button", { id: "cfg-save", class: "btn" }, ["Save"]),
        el("button", { id: "cfg-undo", class: "btn btn-outline" }, ["Undo"]),
        el("button", { id: "cfg-redo", class: "btn btn-outline" }, ["Redo"]),
        el("button", { id: "cfg-reload", class: "btn btn-outline" }, ["Reload from disk"]),
        el("button", { id: "cfg-reset", class: "btn btn-outline" }, ["Reset to defaults"]),
        el("button", { id: "cfg-export", class: "btn btn-outline" }, ["Export JSON"]),
        el("button", { id: "cfg-import", class: "btn btn-outline" }, ["Import JSON"]),
      ]),
      el("div", { class: "row", style: "margin-bottom:12px" }, [
        el("span", { class: "muted", id: "cfg-meta" }),
      ]),
      el("div", { id: "cfg-form", class: "config-grid" }),
    ]),
  );
  v.append(
    el("div", { class: "card" }, [
      el("h3", {}, ["Blocked identifiers"]),
      el("div", { id: "cfg-blocked", class: "history-list" }),
    ]),
  );
  v.append(
    el("div", { class: "card" }, [
      el("h3", {}, ["Change history (click an entry to revert to that state)"]),
      el("div", { id: "cfg-history", class: "history-list" }),
    ]),
  );
  loadConfig();
}

async function loadConfig() {
  try {
    configData = await api("/config");
    renderConfigForm();
  } catch (e: any) {
    snack(e.message);
  }
}

function renderConfigForm() {
  if (!configData) return;
  const c = configData.config;
  const form = $("#cfg-form");
  if (!form) return;
  form.innerHTML = "";
  $("#cfg-meta").textContent =
    `version ${configData.version} · undo ${configData.undo_available ? "available" : "—"} · redo ${configData.redo_available ? "available" : "—"}`;

  interface CfgField {
    path: string;
    label: string;
    kind: "range" | "text" | "select";
    min?: number;
    max?: number;
    step?: number;
    unit?: string;
    prefix?: string;
    options?: [string, string][];
  }
  const groups: [string, string | null, CfgField[]][] = [
    [
      "General",
      null,
      [
        {
          path: "global.permission_file",
          label: "Permissions file",
          kind: "text",
        },
        {
          path: "global.jwt_token_lifetime_minutes",
          label: "JWT lifetime",
          kind: "range",
          min: 30,
          max: 1440,
          step: 30,
          unit: "min",
        },
        {
          path: "auth.max_per_minute_per_ip",
          label: "Client /auth attempts per IP / minute",
          kind: "range",
          min: 1,
          max: 100,
          step: 1,
        },
        {
          path: "auth.session_ttl_hours",
          label: "Admin session TTL",
          kind: "range",
          min: 1,
          max: 72,
          step: 1,
          unit: "h",
        },
      ],
    ],
    [
      "Rate limiting",
      "Every tick (tick_seconds) each app's limit shrinks by 1/(1 + latency_sensitivity·lat_err + pressure_sensitivity·pressure) under load, and grows by growth_rate when healthy. enforced = clamp(round(limit · multiplier · weight), min, max) — the per-app weight is set in Clients.",
      [
        {
          path: "rate_limit.multiplier",
          label: "Master multiplier",
          kind: "range",
          min: 0.1,
          max: 10,
          step: 0.1,
          prefix: "×",
        },
        {
          path: "rate_limit.target_latency_ms",
          label: "Target p50 latency",
          kind: "range",
          min: 10,
          max: 500,
          step: 10,
          unit: "ms",
        },
        {
          path: "rate_limit.latency_sensitivity",
          label: "Latency sensitivity",
          kind: "range",
          min: 0.1,
          max: 10,
          step: 0.1,
        },
        {
          path: "rate_limit.pressure_sensitivity",
          label: "Pressure sensitivity",
          kind: "range",
          min: 0.1,
          max: 10,
          step: 0.1,
        },
        {
          path: "rate_limit.growth_rate",
          label: "Growth rate per tick",
          kind: "range",
          min: 1,
          max: 2,
          step: 0.01,
          prefix: "×",
        },
        {
          path: "rate_limit.min_limit",
          label: "Min docs per page",
          kind: "range",
          min: 1,
          max: 100,
          step: 1,
        },
        {
          path: "rate_limit.max_limit",
          label: "Max docs per page",
          kind: "range",
          min: 10,
          max: 1000,
          step: 10,
        },
        {
          path: "rate_limit.tick_seconds",
          label: "Tick interval",
          kind: "range",
          min: 1,
          max: 60,
          step: 1,
          unit: "s",
        },
        {
          path: "rate_limit.ema_alpha",
          label: "Rate smoothing α",
          kind: "range",
          min: 0.01,
          max: 0.9,
          step: 0.01,
        },
      ],
    ],
    [
      "Dashboard",
      null,
      [
        {
          path: "dashboard.poll_seconds",
          label: "Dashboard poll interval",
          kind: "range",
          min: 0.1,
          max: 10,
          step: 0.1,
          unit: "s",
        },
        {
          path: "dashboard.graph_smoothing",
          label: "Graph smoothing window",
          kind: "range",
          min: 1,
          max: 20,
          step: 1,
          unit: "samples",
        },
        {
          path: "health.cache_ttl_seconds",
          label: "Health cache TTL",
          kind: "range",
          min: 1,
          max: 60,
          step: 1,
          unit: "s",
        },
        {
          path: "dashboard.log_level",
          label: "Log level",
          kind: "select",
          options: [
            ["info", "info"],
            ["debug", "debug (per-request lines)"],
          ],
        },
        {
          path: "dashboard.theme",
          label: "Theme",
          kind: "select",
          options: [
            ["system", "System"],
            ["light", "Light"],
            ["dark", "Dark"],
          ],
        },
      ],
    ],
  ];
  const fmtVal = (f: CfgField, v: number): string => {
    const digits = f.step !== undefined && f.step < 1 ? (f.step < 0.1 ? 2 : 1) : 0;
    return (f.prefix ?? "") + v.toFixed(digits) + (f.unit ? " " + f.unit : "");
  };
  const curVal = (path: string): any => {
    const [g, k] = path.split(".");
    return (c as any)[g][k];
  };
  for (const [group, hint, fields] of groups) {
    const card = el("div", { class: "card", style: "margin-bottom:0" });
    card.append(el("h3", {}, [group]));
    if (hint) card.append(el("p", { class: "hint" }, [hint]));
    for (const f of fields) {
      if (f.kind === "range") {
        const val = Number(curVal(f.path));
        const field = el("div", { class: "sf" });
        const top = el("div", { class: "sf-top" });
        top.append(el("label", {}, [f.label]));
        const out = el("span", { class: "sf-value" }, [fmtVal(f, val)]);
        top.append(out);
        const inp = el("input", {
          type: "range",
          min: String(f.min),
          max: String(f.max),
          step: String(f.step),
          value: String(val),
        }) as HTMLInputElement;
        inp.dataset.path = f.path;
        inp.addEventListener("input", () => {
          out.textContent = fmtVal(f, parseFloat(inp.value));
          markCfgDirty();
        });
        field.append(top, inp);
        card.append(field);
      } else if (f.kind === "select") {
        const field = el("div", { class: "field" });
        field.append(el("label", {}, [f.label]));
        const sel = el("select", { "data-path": f.path });
        for (const [v, l] of f.options!) {
          const opt = el("option", { value: v }, [l]) as HTMLOptionElement;
          opt.selected = String(curVal(f.path)) === v;
          sel.append(opt);
        }
        field.append(sel);
        sel.addEventListener("change", markCfgDirty);
        card.append(field);
      } else {
        const field = el("div", { class: "field" });
        field.append(el("label", {}, [f.label]));
        const inp = el("input", {
          type: "text",
          value: String(curVal(f.path)),
          spellcheck: "false",
        });
        inp.dataset.path = f.path;
        inp.addEventListener("input", markCfgDirty);
        field.append(inp);
        card.append(field);
      }
    }
    form.append(card);
  }

  // blocked list (full-width card below the columns, history-list layout)
  const blk = $("#cfg-blocked");
  blk.innerHTML = "";
  if (c.blocked.length === 0) blk.append(el("div", { class: "muted" }, ["nothing blocked"]));
  for (const id of c.blocked) {
    const row = el("div", {});
    row.append(el("span", { class: "badge bad" }, [esc(id)]));
    const ub = el("button", { class: "btn btn-small btn-outline", style: "margin-left:auto" }, ["unblock"]);
    ub.onclick = async () => {
      await api("/unblock", { method: "POST", body: JSON.stringify({ id }) });
      loadConfig();
    };
    row.append(ub);
    blk.append(row);
  }

  // history
  const hist = $("#cfg-history");
  hist.innerHTML = "";
  if (configData.history.length === 0) hist.append(el("div", { class: "muted" }, ["no changes yet"]));
  configData.history.forEach((h: any, i: number) => {
    const row = el("div", {});
    const desc = el("span", { class: "h-desc" });
    desc.append(h.desc);
    desc.append(el("span", { class: "muted" }, [`(${h.path})`]));
    row.append(desc);
    row.append(el("span", { class: "muted" }, [fmtClock(h.ts * 1000)]));
    row.style.cursor = "pointer";
    row.title = "revert to this state";
    row.onclick = async () => {
      if (
        !(await confirmDialog(
          "Revert config?",
          `Restore the state before "${h.desc}"? Changes after it will be discarded.`,
          "Revert",
        ))
      )
        return;
      try {
        await api("/config/revert", {
          method: "POST",
          body: JSON.stringify({ index: i }),
        });
        snack("reverted");
        loadConfig();
      } catch (e: any) {
        snack(e.message);
      }
    };
    hist.append(row);
  });

  $("#cfg-save").onclick = async () => {
    if (configSaving) return; // in-flight guard: double-save would create duplicate history entries
    const newCfg = structuredClone(c);
    for (const inp of Array.from(form.querySelectorAll("input[data-path], select[data-path]"))) {
      const ip = inp as HTMLInputElement;
      const parts = ip.dataset.path!.split(".");
      if (ip.type === "range") newCfg[parts[0]][parts[1]] = parseFloat(ip.value);
      else if (ip.tagName === "SELECT")
        newCfg[parts[0]][parts[1]] = (ip as unknown as HTMLSelectElement).value;
      else newCfg[parts[0]][parts[1]] = ip.value;
    }
    configSaving = true;
    updateCfgSave();
    try {
      configData = await api("/config", {
        method: "POST",
        body: JSON.stringify({ config: newCfg }),
      });
      snack("config saved");
      renderConfigForm();
    } catch (e: any) {
      snack(e.message);
    } finally {
      configSaving = false;
      updateCfgSave();
    }
  };
  $("#cfg-undo").onclick = async () => {
    const r = await api("/config/undo", { method: "POST" });
    loadConfig();
    snack(r.ok ? "undone" : "nothing to undo");
  };
  $("#cfg-redo").onclick = async () => {
    const r = await api("/config/redo", { method: "POST" });
    loadConfig();
    snack(r.ok ? "redone" : "nothing to redo");
  };
  $("#cfg-reload").onclick = async () => {
    await api("/config/reload", { method: "POST" });
    loadConfig();
    snack("reloaded");
  };
  $("#cfg-reset").onclick = async () => {
    if (
      !(await confirmDialog(
        "Reset to defaults?",
        "All settings return to their default values (history kept).",
        "Reset",
      ))
    )
      return;
    await api("/config/reset", { method: "POST" });
    loadConfig();
    snack("reset to defaults");
  };
  $("#cfg-export").onclick = async () => {
    const r = await fetch("/dashboard/api/config/export", {
      credentials: "same-origin",
    });
    if (!r.ok) {
      const b = await r.json().catch(() => ({}));
      snack(b.error || "export failed");
      return;
    }
    const blob = await r.blob();
    const a = el("a", {
      href: URL.createObjectURL(blob),
      download: "config.json",
    });
    a.click();
  };
  $("#cfg-import").onclick = () => {
    const box = $("#dialog-box");
    box.innerHTML = "";
    box.append(el("h3", {}, ["Import config (JSON)"]));
    box.append(
      el("textarea", {
        id: "cfg-import-text",
        style: "width:100%;height:180px;font-family:monospace;font-size:12px",
      }),
    );
    const actions = el("div", { class: "dialog-actions" });
    const cancel = el("button", { class: "btn btn-text" }, ["Cancel"]);
    cancel.onclick = () => $("#dialog").classList.add("hidden");
    const ok = el("button", { class: "btn" }, ["Import"]);
    ok.onclick = async () => {
      try {
        const parsed = JSON.parse(($("#cfg-import-text") as HTMLTextAreaElement).value);
        await api("/config/import", {
          method: "POST",
          body: JSON.stringify({ config: parsed }),
        });
        $("#dialog").classList.add("hidden");
        loadConfig();
        snack("imported");
      } catch (e: any) {
        snack("import failed: " + e.message);
      }
    };
    actions.append(cancel, ok);
    box.append(actions);
    $("#dialog").classList.remove("hidden");
  };

  configDirty = false;
  updateCfgSave();
}

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

async function renderLogs() {
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
      el("div", { class: "logs-box", id: "logs-box" }),
    ]),
  );

  const box = $("#logs-box") as HTMLElement;
  const load = async () => {
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
  const fetchOlder = async () => {
    if (logBusy || logNoMore || logOldestSeq <= 0) return;
    logBusy = true;
    const before = logOldestSeq;
    const h0 = box.scrollHeight;
    try {
      const d = await logsFetch(before);
      logTotal = d.total;
      if (d.lines.length === 0) {
        logNoMore = true;
        return;
      }
      logApps = d.apps;
      logNames = d.names;
      logLoggers = d.loggers;
      renderLogSugg();
      const seen = new Set(logLoaded.map((e: any) => e.seq));
      const fresh = d.lines.filter((e: any) => !seen.has(e.seq));
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
        if (logMatches(e, re)) frag.append(logRow(e));
      }
      box.insertBefore(frag, box.firstChild);
      box.scrollTop += box.scrollHeight - h0;
    } catch (e: any) {
      snack(e.message);
    } finally {
      logBusy = false;
    }
  };
  box.addEventListener("scroll", () => {
    if (box.scrollTop < 40) fetchOlder();
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
    renderLogList();
    renderLogSugg();
    setHint();
  };
  load();
}

/* ============================= boot ============================= */

function applyTheme(theme: string) {
  const t = theme || "system";
  document.documentElement.dataset.theme =
    t === "system" ? (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light") : t;
}

function initShell() {
  $("#logout-btn").onclick = async () => {
    await fetch("/dashboard/api/logout", {
      method: "POST",
      credentials: "same-origin",
    });
    stopPolling();
    showLogin();
  };
  $("#mongo-btn").onclick = async () => {
    try {
      const h = await refreshMongoStatus();
      const st = h?.status ?? "unhealthy";
      const m = h?.mongodb ?? {};
      const ping = st === "unhealthy" ? "" : ` — ping ${fmtNum(m.ping_latency_ms ?? 0, 1)} ms`;
      snack(`MongoDB ${st === "ok" ? "reachable" : st}${ping}${m.error ? ` — ${m.error}` : ""}`);
    } catch {
      snack("health check failed");
    }
  };
  $("#mongo-refresh").onclick = async () => {
    try {
      await refreshMongoStatus();
    } catch {}
  };
  $("#theme-btn").onclick = () => {
    const cur = document.documentElement.dataset.theme;
    const next = cur === "dark" ? "light" : "dark";
    document.documentElement.dataset.theme = next;
    localStorage.setItem("xdb-theme", next);
  };
  const saved = localStorage.getItem("xdb-theme");
  if (saved) document.documentElement.dataset.theme = saved;
  else applyTheme("system");
  // live theme sync with the config
  setInterval(async () => {
    if (!lastMetrics || currentRoute !== "overview") return;
    applyTheme(lastMetrics.config.theme === "system" ? "system" : lastMetrics.config.theme);
  }, 5000);
}

window.addEventListener("load", () => {
  initShell();
  initLogin();
  checkSession();
});
