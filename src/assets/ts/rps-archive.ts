// Long-window RPS history for the overview "all apps" chart. The dashboard
// samples every /metrics poll and downsamples into tiered time buckets,
// persisted in localStorage — windows up to a year survive reloads. Coverage
// is limited to times the dashboard was open (gaps compress, they don't
// interpolate).
//
// Tiers are independent of the backend tick: the finest tier buckets at 1 s
// (the practical cap is the dashboard poll interval, default 2 s), so point
// density follows whatever rate data actually arrives at. `window()` reads
// the finest tier covering the X window, then re-bins to at most
// RPS_TARGET_POINTS points — with a 5 s backend tick a 10-minute window tops
// out below target, with a faster tick it reaches it.
export const RPS_TARGET_POINTS = 300;
export const RPS_TIERS: [resSec: number, keepSec: number][] = [
  [1, 600], // 10 min @ 1 s
  [4, 2400], // 40 min @ 4 s
  [12, 3600], // 1 h @ 12 s
  [60, 36000], // 10 h @ 1 min
  [300, 180000], // 50 h @ 5 min
  [900, 259200], // 3 d @ 15 min
  [1800, 1209600], // 2 w @ 30 min
  [7200, 5259600], // 2 mo @ 2 h
  [21600, 7889400], // 3 mo @ 6 h
  [86400, 34560000], // 400 d @ 1 d
];
export const RPS_WINDOWS: [label: string, sec: number][] = [
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
export const getRpsWindowIdx = (): number => rpsWindowIdx;
export const setRpsWindowIdx = (i: number): void => {
  if (i >= 0 && i < RPS_WINDOWS.length) {
    rpsWindowIdx = i;
    localStorage.setItem(RPS_WINDOW_LS, String(i));
  }
};

interface RpsTier {
  ts: number[];
  vs: number[];
  open: { t: number; sum: number; n: number } | null;
}
const RPS_ARCHIVE_LS = "xdb-rps-archive-v2";
const RPS_ARCHIVE_V1_LS = "xdb-rps-archive-v1";

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
    try {
      localStorage.removeItem(RPS_ARCHIVE_V1_LS); // superseded layout: discard, start blank
    } catch {
      /* storage unavailable */
    }
    return a;
  }

  get startSec(): number {
    return this.firstT;
  }

  /* `apps` is structurally typed (app id + current rps + optional per-name
     rates) to avoid a cycle with state.ts — RpsArchive has no runtime
     dependency on Metrics. name_id series are stored under the same map
     with the server's "name:<id>@<app>" key convention. */
  sample(apps: { app: string; rps: number; names?: { name: string; rps: number }[] }[], nowMs: number) {
    const tSec = Math.floor(nowMs / 1000);
    if (!this.firstT) this.firstT = tSec;
    this.dirty = true;
    for (const a of apps) {
      this.pushSample(a.app, Math.max(0, a.rps), tSec);
      for (const n of a.names ?? []) this.pushSample(`name:${n.name}@${a.app}`, Math.max(0, n.rps), tSec);
    }
    if (Date.now() - this.lastSaveMs > 30000) this.save();
  }

  private pushSample(key: string, v: number, tSec: number) {
    let s = this.series[key];
    if (!s) {
      s = { lastT: tSec, tiers: RPS_TIERS.map(() => ({ ts: [], vs: [], open: null })) };
      this.series[key] = s;
    }
    s.lastT = tSec;
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

  /* buckets of the finest tier covering `windowSec` (closed + the open one),
     re-binned to at most RPS_TARGET_POINTS points across the window — works
     for any series key: app ids and "name:<id>@<app>" breakdown keys */
  window(keys: string[], windowSec: number, nowSec: number): Map<string, { t: number; v: number }[]> {
    let ti = RPS_TIERS.findIndex(([, keep]) => keep >= windowSec);
    if (ti < 0) ti = RPS_TIERS.length - 1;
    const res = RPS_TIERS[ti][0];
    const t0 = nowSec - windowSec;
    const out = new Map<string, { t: number; v: number }[]>();
    for (const key of keys) {
      const tier = this.series[key]?.tiers[ti];
      const pts: { t: number; v: number }[] = [];
      if (tier) {
        for (let i = 0; i < tier.ts.length; i++)
          if (tier.ts[i] + res > t0) pts.push({ t: tier.ts[i] + res / 2, v: tier.vs[i] });
        if (tier.open && tier.open.n) pts.push({ t: tier.open.t + res / 2, v: tier.open.sum / tier.open.n });
      }
      out.set(key, rebin(pts, RPS_TARGET_POINTS, t0, nowSec));
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

/* re-bin `pts` into at most `target` equal-width time bins over [t0, t1]:
   each output point is the average of the bin's points, timestamped at the
   bin center. Empty bins are skipped, so gaps stay gaps. Returns `pts`
   unchanged when it already fits the target. */
function rebin(
  pts: { t: number; v: number }[],
  target: number,
  t0: number,
  t1: number,
): { t: number; v: number }[] {
  if (pts.length <= target || t1 <= t0) return pts;
  const w = (t1 - t0) / target;
  const acc: { s: number; n: number }[] = Array.from({ length: target }, () => ({ s: 0, n: 0 }));
  for (const p of pts) {
    let b = Math.floor((p.t - t0) / w);
    if (b < 0) b = 0;
    else if (b >= target) b = target - 1;
    acc[b].s += p.v;
    acc[b].n++;
  }
  const out: { t: number; v: number }[] = [];
  for (let b = 0; b < target; b++) if (acc[b].n) out.push({ t: t0 + (b + 0.5) * w, v: acc[b].s / acc[b].n });
  return out;
}

export const rpsArchive = RpsArchive.load();
window.addEventListener("beforeunload", () => {
  rpsArchive.flushIfDirty();
});
