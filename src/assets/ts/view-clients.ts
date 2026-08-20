// Clients & permissions tab: the app/name tree with live rps sparklines,
// block/unblock, weight popover, permission panels, adaptive limits, cursors.
import { $, el, esc, fmtNum, timeAgo, snack, confirmDialog } from "./core";
import { sparkline, getCss } from "./charts";
import { api, poll, lastMetrics, Metrics, AppNode, ClientNode } from "./state";
import { updateMongoStatus } from "./mongo";
import {
  renderPermWidget,
  renderEffective,
  runCheck,
  ACTIONS,
  Rule,
  EffectiveRule,
  NamePerm,
  AppPerm,
} from "./perm-widget";

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

export function renderClients() {
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

export function renderClientsData(m: Metrics) {
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
  renderPermWidget(
    widget,
    entry.allow,
    entry.deny,
    { scope: "app", eff: entry.effective ?? [], dbs: dbList, dbsUnavailable: dbListUnavailable },
    () => queuePermsSave(),
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
  renderPermWidget(
    widget,
    entry.allow,
    entry.deny,
    { scope: "name", eff: entry.effective ?? [], dbs: dbList, dbsUnavailable: dbListUnavailable },
    () => queuePermsSave(),
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
