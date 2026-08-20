// XavierDB dashboard — shared DOM & formatting helpers, snackbar, confirm dialog.
// No app logic here; imported by every other module.
export const $ = <T extends HTMLElement = HTMLElement>(sel: string): T => document.querySelector(sel) as T;
export const $$ = (sel: string): HTMLElement[] => Array.from(document.querySelectorAll(sel));

export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Record<string, string> = {},
  children: (Node | string)[] = [],
): HTMLElementTagNameMap[K] {
  const n = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) n.setAttribute(k, v);
  for (const c of children) n.append(c as Node);
  return n;
}

export function esc(s: unknown): string {
  return String(s).replace(
    /[&<>"']/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c] as string,
  );
}

export function fmtNum(v: number, digits = 1): string {
  if (!isFinite(v)) return "—";
  if (Math.abs(v) >= 1000) return v.toFixed(0);
  return v.toFixed(digits);
}

export function fmtBytes(mb: number): string {
  if (mb >= 1024) return (mb / 1024).toFixed(1) + " GB";
  return mb.toFixed(0) + " MB";
}

export function timeAgo(ms: number): string {
  if (!ms) return "never";
  const s = Math.max(0, (Date.now() - ms) / 1000);
  if (s < 5) return "now";
  if (s < 60) return fmtNum(s, 0) + "s ago";
  if (s < 3600) return fmtNum(s / 60, 0) + "m ago";
  return fmtNum(s / 3600, 1) + "h ago";
}

export function fmtClock(ts: number): string {
  const d = new Date(ts);
  return d.toLocaleTimeString();
}

export function fmtUptime(s: number): string {
  if (s < 60) return fmtNum(s, 0) + "s";
  if (s < 3600) return fmtNum(s / 60, 0) + "m";
  if (s < 86400) return fmtNum(s / 3600, 1) + "h";
  return fmtNum(s / 86400, 1) + "d";
}

let snackTimer = 0;
export function snack(msg: string, ms = 2600) {
  const s = $("#snackbar");
  s.textContent = msg;
  s.classList.add("show");
  clearTimeout(snackTimer);
  snackTimer = window.setTimeout(() => s.classList.remove("show"), ms);
}

export function confirmDialog(title: string, body: string, okLabel = "Confirm"): Promise<boolean> {
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
