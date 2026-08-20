// Long-window RPS history for the overview "all apps" chart. The server only
// keeps ~120 ticks (~10 min) of sparkline history, so the dashboard samples
// every /metrics poll and downsamples into tiered time buckets, persisted in
// localStorage — windows up to a year survive reloads. Coverage is limited
// to times the dashboard was open (gaps compress, they don't interpolate).
// to times the dashboard was open (gaps compress, they don't interpolate).
export const RPS_TIERS: [resSec: number, keepSec: number][] = [
  [10, 1800], // 30 min @ 10 s
  [60, 10800], // 3 h @ 1 min
  [300, 43200], // 12 h @ 5 min
  [1800, 259200], // 3 d @ 30 min
  [21600, 1814400], // 21 d @ 6 h
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

  /* `apps` is structurally typed (app id + current rps) to avoid a cycle
     with state.ts — RpsArchive has no runtime dependency on Metrics. */
  sample(apps: { app: string; rps: number }[], nowMs: number) {
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
        if (tier.open && tier.open.n) pts.push({ t: tier.open.t + res / 2, v: tier.open.sum / tier.open.n });
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
export const rpsArchive = RpsArchive.load();
window.addEventListener("beforeunload", () => {
  rpsArchive.flushIfDirty();
});
