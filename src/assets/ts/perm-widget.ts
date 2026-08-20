// The permission editor widget (allow/deny/inherit action badges per
// database & collection) + the effective-rules table. Scope-agnostic: works
// for both app and name permission panels (view-clients drives it).
import { el, esc, snack } from "./core";

export interface Rule {
  actions: string[];
  databases: string[];
  collections: string[];
  source?: string;
}
export interface NamePerm {
  name: string;
  allow: Rule[];
  deny: Rule[];
  effective?: EffectiveRule[];
  delete?: boolean;
}
export interface AppPerm {
  app: string;
  token_set: boolean;
  allow: Rule[];
  deny: Rule[];
  effective?: EffectiveRule[];
  names: NamePerm[];
  delete?: boolean;
  set_token?: string;
}
export interface EffectiveRule {
  source: string;
  actions: string[];
  databases: string[];
  collections: string[];
}

export const ACTIONS = ["GET", "POST", "PUT", "PATCH", "DELETE", "INDEX"];

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
  // live MongoDB listing, passed by view-clients (keeps this module cycle-free)
  dbs?: { name: string; collections: string[] }[];
  dbsUnavailable?: boolean;
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

export function glob(pattern: string, value: string): boolean {
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
export function effVerdict(
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

export function renderPermWidget(
  cont: HTMLElement,
  allow: Rule[],
  deny: Rule[],
  ctx: PermCtx,
  onSave: () => void,
) {
  const dbList = ctx.dbs ?? [];
  const dbListUnavailable = !!ctx.dbsUnavailable;
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

export function renderEffective(cont: HTMLElement, eff: EffectiveRule[], appScope: boolean) {
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

export function runCheck(db: string, coll: string, out: HTMLElement, eff: EffectiveRule[]) {
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
