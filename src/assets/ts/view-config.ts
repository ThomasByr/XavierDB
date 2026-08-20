// Config tab: binary `config` form (sliders/selects/text), blocked list,
// change history with revert, undo/redo, JSON export/import.
import { $, el, esc, snack, confirmDialog, fmtClock } from "./core";
import { api } from "./state";

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

export function renderConfig() {
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
