// XavierDB dashboard — vanilla TypeScript, zero libraries.
// Hash-routed SPA: overview / clients / config / logs.
// Entry point: theme, topbar shell wiring, login boot. All views and state
// live in the view-*/state modules; see .agents/knowledge/architecture.md.

import { $, el, fmtNum, snack } from "./core";
import {
  stopPolling,
  showLogin,
  checkSession,
  initLogin,
  getChartSmoothing,
  setChartSmoothing,
  getPollInterval,
  setPollInterval,
} from "./state";
import { refreshMongoStatus } from "./mongo";
/* ============================= boot ============================= */

function applyTheme(t: string) {
  document.documentElement.dataset.theme = t;
  const btn = $("#theme-btn");
  if (t === "dark") btn.setAttribute("data-dark", "");
  else btn.removeAttribute("data-dark");
  $("#theme-label").textContent = t === "dark" ? "Dark" : "Light";
}

/* Settings popover: per-browser display preferences (localStorage):
   chart smoothing coefficient + metrics poll interval. */
let settingsPop: HTMLElement | null = null;

function closeSettingsPop() {
  settingsPop?.remove();
  settingsPop = null;
  document.removeEventListener("mousedown", settingsDocHandler);
}
function settingsDocHandler(ev: MouseEvent) {
  const t = ev.target as Node;
  if (settingsPop && !settingsPop.contains(t) && !$("#settings-btn").contains(t)) {
    closeSettingsPop();
  }
}

function sliderRow(
  label: string,
  min: number,
  max: number,
  step: number,
  value: number,
  fmt: (v: number) => string,
  onApply: (v: number) => void,
): HTMLElement {
  const row = el("div", { class: "sp-row" });
  const top = el("div", { class: "sp-top" });
  top.append(el("label", {}, [label]));
  const out = el("span", { class: "sp-val" }, [fmt(value)]);
  top.append(out);
  const inp = el("input", {
    type: "range",
    min: String(min),
    max: String(max),
    step: String(step),
    value: String(value),
  }) as HTMLInputElement;
  inp.addEventListener("input", () => {
    out.textContent = fmt(parseFloat(inp.value));
  });
  inp.addEventListener("change", () => onApply(parseFloat(inp.value)));
  row.append(top, inp);
  return row;
}

function openSettingsPop() {
  closeSettingsPop();
  const pop = el("div", { class: "settings-pop elevation-3" });
  pop.append(
    el("h3", {}, ["Dashboard settings"]),
    sliderRow("Chart smoothing", 0, 1, 0.01, getChartSmoothing(), (v) => v.toFixed(2), setChartSmoothing),
    sliderRow("Poll interval", 0.1, 10, 0.1, getPollInterval(), (v) => `${v.toFixed(1)} s`, setPollInterval),
    el("p", { class: "sp-hint" }, ["Saved in this browser (localStorage) — not server config."]),
  );
  document.body.append(pop);
  settingsPop = pop;
  document.addEventListener("mousedown", settingsDocHandler);
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
    const next = document.documentElement.dataset.theme === "dark" ? "light" : "dark";
    localStorage.setItem("xdb-theme", next);
    applyTheme(next);
  };
  $("#settings-btn").onclick = () => {
    if (settingsPop) closeSettingsPop();
    else openSettingsPop();
  };
  // Theme is purely client-side: dark by default, toggled via the button.
  applyTheme(localStorage.getItem("xdb-theme") || "dark");
}

window.addEventListener("load", () => {
  initShell();
  initLogin();
  checkSession();
});
