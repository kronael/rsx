import type { SymbolMeta } from "../lib/types";
import type { UserPosition } from "../lib/types";
import type { UserOrder } from "../lib/types";
import type { UserFill } from "../lib/types";

const API_BASE = typeof location !== "undefined"
  ? location.pathname.replace(/\/trade\/.*$/, "")
  : "";

async function apiFetch<T>(path: string): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    signal: AbortSignal.timeout(10000),
  });
  if (!res.ok) {
    throw new Error(`${res.status} ${res.statusText}`);
  }
  return res.json() as Promise<T>;
}

function qs(
  params: Record<string, string | number | undefined>,
): string {
  const parts: string[] = [];
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined) {
      parts.push(
        `${encodeURIComponent(k)}=${encodeURIComponent(v)}`,
      );
    }
  }
  return parts.length > 0 ? `?${parts.join("&")}` : "";
}

interface MetadataResponse {
  M: [number, string, string, string][];
}

export async function fetchSymbols(): Promise<SymbolMeta[]> {
  const data = await apiFetch<MetadataResponse>(
    "/v1/symbols",
  );
  return data.M.map(([id, tick, lot, name]) => ({
    id,
    name,
    tickSize: parseFloat(tick),
    lotSize: parseFloat(lot),
  }));
}

interface AccountResponse {
  collateral: number;
  equity: number;
  pnl: number;
  im: number;
  mm: number;
  available: number;
}

export async function fetchAccount(): Promise<
  AccountResponse
> {
  return apiFetch<AccountResponse>("/v1/account");
}

export async function fetchPositions(): Promise<
  UserPosition[]
> {
  return apiFetch<UserPosition[]>("/v1/positions");
}

export async function fetchOrders(): Promise<UserOrder[]> {
  return apiFetch<UserOrder[]>("/v1/orders");
}

export async function fetchFills(
  sym?: number,
  limit?: number,
  before?: string,
): Promise<UserFill[]> {
  return apiFetch<UserFill[]>(
    `/v1/fills${qs({ sym, limit, before })}`,
  );
}

interface FundingEntry {
  ts: number;
  symbolId: number;
  amount: number;
  rate: number;
}

export async function fetchFunding(
  sym?: number,
  limit?: number,
  before?: string,
): Promise<FundingEntry[]> {
  return apiFetch<FundingEntry[]>(
    `/v1/funding${qs({ sym, limit, before })}`,
  );
}
