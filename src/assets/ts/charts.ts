// Canvas drawing primitives: sparklines, mini area charts, stable per-app colors.
// Pure rendering — no state, no fetches.
export function sparkline(canvas: HTMLCanvasElement, data: number[], color: string, max = 0): void {
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth || 120;
  const h = canvas.clientHeight || 28;
  canvas.width = w * dpr;
  canvas.height = h * dpr;
  const ctx = canvas.getContext("2d")!;
  ctx.scale(dpr, dpr);
  ctx.clearRect(0, 0, w, h);
  if (data.length < 2) return;
  const hi = max > 0 ? max : Math.max(...data, 1e-9);
  const step = w / (data.length - 1);
  ctx.strokeStyle = color;
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  data.forEach((v, i) => {
    const x = i * step;
    const y = h - 2 - (Math.max(0, v) / hi) * (h - 4);
    i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
  });
  ctx.stroke();
}

// stable per-app line colors (hash of the id — survives re-renders and windows)
const LINE_PALETTE = [
  "#6d4aff",
  "#00897b",
  "#e53935",
  "#f9a825",
  "#3949ab",
  "#8e24aa",
  "#00acc1",
  "#6d4c41",
  "#43a047",
  "#f4511e",
  "#546e7a",
  "#c2185b",
];

export function lineColor(app: string): string {
  let h = 5381;
  for (let i = 0; i < app.length; i++) h = ((h * 33) ^ app.charCodeAt(i)) >>> 0;
  return LINE_PALETTE[h % LINE_PALETTE.length];
}

export function getCss(v: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(v).trim() || "#6d4aff";
}

/* #rrggbb + alpha → rgba() string (palette colors are hex; alpha for
   stacked name_id lines/bands and the hover crosshair) */
export function withAlpha(hex: string, a: number): string {
  const h = hex.replace("#", "");
  const full = h.length === 3 ? h.split("").map((c) => c + c).join("") : h;
  const r = parseInt(full.slice(0, 2), 16) || 0;
  const g = parseInt(full.slice(2, 4), 16) || 0;
  const b = parseInt(full.slice(4, 6), 16) || 0;
  const al = Math.round(Math.max(0, Math.min(1, a)) * 1000) / 1000;
  return `rgba(${r},${g},${b},${al})`;
}

/* single-line mini chart with a soft area fill — no legend, no axis text */
export function drawMini(canvas: HTMLCanvasElement, data: number[], color: string) {
  if (!canvas) return;
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth || 200;
  const h = canvas.clientHeight || 48;
  canvas.width = w * dpr;
  canvas.height = h * dpr;
  const ctx = canvas.getContext("2d")!;
  ctx.scale(dpr, dpr);
  ctx.clearRect(0, 0, w, h);
  if (data.length < 2) return;
  const hi = Math.max(...data, 1e-9) * 1.08;
  const step = w / (data.length - 1);
  ctx.beginPath();
  data.forEach((v, i) => {
    const x = i * step;
    const y = h - 2 - (Math.max(0, v) / hi) * (h - 4);
    i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
  });
  ctx.strokeStyle = color;
  ctx.lineWidth = 1.75;
  ctx.lineJoin = "round";
  ctx.stroke();
  ctx.globalAlpha = 0.12;
  ctx.lineTo(w, h);
  ctx.lineTo(0, h);
  ctx.closePath();
  ctx.fillStyle = color;
  ctx.fill();
  ctx.globalAlpha = 1;
}
