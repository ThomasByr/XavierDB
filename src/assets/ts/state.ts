// App session & routing core: the /dashboard/api fetch wrapper, login view
// switching, hash router, and the metrics poll loop. `lastMetrics` lives here.
// NOTE: deliberate import cycle with the view-* modules (router → views →
// api/lastMetrics) — safe: every cross-module use happens at call time, never
// during module evaluation.
import { $, $$ } from "./core";
import { rpsArchive } from "./rps-archive";
import { renderOverview, renderOverviewData } from "./view-overview";
import { renderClients, renderClientsData } from "./view-clients";
import { renderConfig } from "./view-config";
import { renderLogs, releaseLogs } from "./view-logs";

export async function api(path: string, opts: RequestInit = {}): Promise<any> {
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
/* client-side preferences, persisted in localStorage (per browser):
   graph smoothing window (samples) + metrics poll interval (seconds). */
export function getSmoothingWindow(): number {
  const v = parseInt(localStorage.getItem("xdb-smoothing") || "", 10);
  return Number.isInteger(v) && v >= 1 && v <= 60 ? v : 5;
}
export function setSmoothingWindow(v: number) {
  localStorage.setItem("xdb-smoothing", String(v));
  reseedSmoothers();
}
export function getPollInterval(): number {
  const v = parseFloat(localStorage.getItem("xdb-poll") || "");
  return isFinite(v) && v >= 0.1 && v <= 60 ? v : 2;
}
export function setPollInterval(v: number) {
  localStorage.setItem("xdb-poll", String(v));
  restartPolling();
}

/* ============================= state ============================= */

export interface ClientNode {
  name: string;
  id: string;
  blocked: boolean;
  rps: number;
  p50_ms: number;
  total_requests: number;
  last_seen_ms: number;
  rps_history: number[];
}
export interface AppNode {
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
export interface Metrics {
  ts: number;
  config: {
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

export let lastMetrics: Metrics | null = null;
export let systemSeries: Record<string, Smoother> = {};
export let systemHistory: Record<string, number[]> = {
  cpu: [],
  mem: [],
  disk: [],
  rx: [],
  tx: [],
};

export function seedSmoothers() {
  if (Object.keys(systemSeries).length === 0) reseedSmoothers();
}
function reseedSmoothers() {
  const win = getSmoothingWindow();
  for (const k of Object.keys(systemSeries)) delete systemSeries[k];
  for (const k of Object.keys(systemHistory)) systemSeries[k] = new Smoother(win);
}

export function showLogin() {
  $("#app").classList.add("hidden");
  $("#login-view").classList.remove("hidden");
  $("#login-error").textContent = "";
}

export function showApp() {
  $("#login-view").classList.add("hidden");
  $("#app").classList.remove("hidden");
  route();
}

export async function checkSession() {
  try {
    await api("/session");
    showApp();
  } catch {
    showLogin();
  }
}

export async function initLogin() {
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

/// Optional per-view cleanup when switching away from its tab. A view only
/// needs an entry here if it holds memory that should not outlive being on
/// that tab — e.g. logs: releaseLogs frees the client-side retained ring
/// (filters are kept, so the user's filtered view is restored on return).
/// The router calls this BEFORE rendering the incoming view.
const leaves: Partial<Record<string, () => void>> = {
  logs: releaseLogs,
};

export let currentRoute = "overview";
let pollTimer = 0;
let pollEnabled = true;

export function route() {
  const hash = location.hash.replace(/^#\//, "").split("?")[0] || "overview";
  const prev = currentRoute;
  currentRoute = routes[hash] ? hash : "overview";
  $$(".nav-item").forEach((n) => n.classList.toggle("active", n.getAttribute("data-route") === currentRoute));
  $("#page-title").textContent = currentRoute[0].toUpperCase() + currentRoute.slice(1);
  // Leave hook runs before the incoming view renders, so the outgoing tab's
  // memory is freed before the new one allocates. No-op on first boot or a
  // same-route re-entry (prev === currentRoute).
  if (prev !== currentRoute) leaves[prev]?.();
  routes[currentRoute]();
  // metrics are polled on every tab — views render only on their own route,
  // but the RPS archive (overview chart) must keep sampling everywhere
  pollEnabled = true;
  restartPolling();
}

export function stopPolling() {
  clearInterval(pollTimer);
  pollEnabled = false;
}

export function restartPolling() {
  clearInterval(pollTimer);
  if (pollEnabled) {
    pollTimer = window.setInterval(poll, Math.max(0.1, getPollInterval()) * 1000);
    poll();
  }
}

export async function poll() {
  if (!pollEnabled) return;
  try {
    const m: Metrics = await api("/metrics");
    lastMetrics = m;
    seedSmoothers();
    rpsArchive.sample(m.apps, Date.now());
    if (currentRoute === "overview") renderOverviewData(m);
    if (currentRoute === "clients") renderClientsData(m);
  } catch {
    /* session handled in api() */
  }
}

window.addEventListener("hashchange", route);
