// Price/qty formatting utilities for fixed-point i64

export function formatPrice(
  raw: number,
  tickSize: number,
): string {
  if (!Number.isFinite(raw) || tickSize <= 0) {
    return "--";
  }
  const human = raw * tickSize;
  const decimals = countDecimals(tickSize);
  return human.toFixed(decimals);
}

export function formatQty(
  raw: number,
  lotSize: number,
): string {
  if (!Number.isFinite(raw) || lotSize <= 0) {
    return "--";
  }
  const human = raw * lotSize;
  const decimals = countDecimals(lotSize);
  return human.toFixed(decimals);
}

export function parsePrice(
  human: string,
  tickSize: number,
): number {
  const val = parseFloat(human);
  if (isNaN(val)) return 0;
  return Math.round(val / tickSize);
}

export function parseQty(
  human: string,
  lotSize: number,
): number {
  const val = parseFloat(human);
  if (isNaN(val)) return 0;
  return Math.round(val / lotSize);
}

export function formatPnl(
  raw: number,
  tickSize: number,
): { text: string; positive: boolean } {
  if (!Number.isFinite(raw) || tickSize <= 0) {
    return { text: "--", positive: true };
  }
  const human = raw * tickSize;
  const decimals = countDecimals(tickSize);
  const positive = human >= 0;
  const sign = positive ? "+" : "";
  return {
    text: `${sign}${human.toFixed(decimals)}`,
    positive,
  };
}

export function formatTs(ns: number): string {
  if (ns <= 0 || !Number.isFinite(ns)) return "--:--:--";
  const ms = ns / 1_000_000;
  const d = new Date(ms);
  if (isNaN(d.getTime())) return "--:--:--";
  const h = d.getHours().toString().padStart(2, "0");
  const m = d.getMinutes().toString().padStart(2, "0");
  const s = d.getSeconds().toString().padStart(2, "0");
  return `${h}:${m}:${s}`;
}

export function generateCid(): string {
  const raw = crypto.randomUUID().replace(/-/g, "");
  return raw.slice(0, 20);
}

function countDecimals(val: number): number {
  const str = val.toString();
  if (str.includes("e-")) {
    return parseInt(str.split("e-")[1] ?? "0", 10);
  }
  const dot = str.indexOf(".");
  if (dot < 0) return 0;
  return str.length - dot - 1;
}
