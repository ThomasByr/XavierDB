// Topbar MongoDB status widget — shared by overview, clients and the shell.
import { $, fmtNum } from "./core";
import { lastMetrics } from "./state";

export function updateMongoStatus(h: any) {
  const st = h?.status ?? "unhealthy";
  const m = h?.mongodb ?? {};
  const dot = $("#mongo-dot");
  if (dot) dot.className = "dot " + (st === "ok" ? "ok" : st === "degraded" ? "warn" : "bad");
  const btn = $("#mongo-btn");
  if (!btn) return;
  btn.title =
    st === "ok"
      ? `MongoDB reachable — ping ${fmtNum(m.ping_latency_ms ?? 0, 1)} ms`
      : st === "degraded"
        ? `MongoDB degraded — ping ${fmtNum(m.ping_latency_ms ?? 0, 1)} ms`
        : `MongoDB DOWN — ${m.error ?? "unreachable"}`;
}

export async function refreshMongoStatus(): Promise<any> {
  const res = await fetch("/health");
  const h = await res.json().catch(() => ({}));
  if (lastMetrics) lastMetrics.health = h;
  updateMongoStatus(h);
  return h;
}
