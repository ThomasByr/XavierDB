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

/* client-side preferences, persisted in localStorage (per browser):
   chart smoothing coefficient (0..1, TensorBoard-style EMA — see
   charts.ts emaSmooth) + metrics poll interval (seconds). Separate key
   from the old "xdb-smoothing" window setting so stale 1..20 values
   can't collide with the 0..1 range. */
export function getChartSmoothing(): number {
  const v = parseFloat(localStorage.getItem("xdb-smoothing-alpha") || "");
  return isFinite(v) && v >= 0 && v <= 1 ? v : 0.6;
}
export function setChartSmoothing(v: number) {
  localStorage.setItem("xdb-smoothing-alpha", String(v));
  // smoothing is applied at draw time — re-render the overview now so the
  // change is visible immediately instead of at the next poll
  if (lastMetrics && currentRoute === "overview") renderOverviewData(lastMetrics);
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
/* RAW (unsmoothed) per-key history for the overview mini charts — the EMA
   is applied at draw time (charts.ts emaSmooth) so the smoothing slider
   re-smooths history instantly. */
export let systemHistory: Record<string, number[]> = {
  cpu: [],
  mem: [],
  disk: [],
  rx: [],
  tx: [],
};

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
    rpsArchive.sample(m.apps, Date.now());
    if (currentRoute === "overview") renderOverviewData(m);
    if (currentRoute === "clients") renderClientsData(m);
  } catch {
    /* session handled in api() */
  }
}

window.addEventListener("hashchange", route);
