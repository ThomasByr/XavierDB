// XavierDB dashboard — vanilla TypeScript, zero libraries.
// Hash-routed SPA: overview / clients / config / logs.
// Entry point: theme, topbar shell wiring, login boot. All views and state
// live in the view-*/state modules; see .agents/knowledge/architecture.md.

import { $, fmtNum, snack } from "./core";
import { lastMetrics, currentRoute, stopPolling, showLogin, checkSession, initLogin } from "./state";
import { refreshMongoStatus } from "./mongo";
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
