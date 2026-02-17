import { create } from "zustand";
import { useShallow } from "zustand/react/shallow";
import { useMemo } from "react";
import type {
  SymbolMeta,
  OrderbookState,
  BboState,
  RecentTrade,
  PriceLevel,
  Stats24h,
} from "../lib/types";
import { Side } from "../lib/protocol";

// Fixed-capacity ring buffer: O(1) push, no array copies.
const RING_CAP = 100;

export class TradeRing {
  private buf: (RecentTrade | undefined)[];
  private head: number; // next write slot
  private size: number;

  constructor() {
    this.buf = new Array(RING_CAP);
    this.head = 0;
    this.size = 0;
  }

  push(t: RecentTrade): TradeRing {
    const next = new TradeRing();
    next.buf = this.buf.slice(); // shallow copy of fixed array
    next.head = this.head;
    next.size = this.size;
    next.buf[next.head] = t;
    next.head = (next.head + 1) % RING_CAP;
    if (next.size < RING_CAP) next.size++;
    return next;
  }

  // Returns trades newest-first.
  snapshot(): RecentTrade[] {
    const out: RecentTrade[] = [];
    for (let i = 0; i < this.size; i++) {
      const idx = (this.head - 1 - i + RING_CAP) % RING_CAP;
      const t = this.buf[idx];
      if (t !== undefined) out.push(t);
    }
    return out;
  }

  newest(): RecentTrade | undefined {
    if (this.size === 0) return undefined;
    return this.buf[(this.head - 1 + RING_CAP) % RING_CAP];
  }

  get length(): number {
    return this.size;
  }

  clear(): TradeRing {
    return new TradeRing();
  }
}

interface MarketStore {
  symbols: Map<number, SymbolMeta>;
  selectedSymbol: number;
  orderbook: OrderbookState;
  bbo: BboState;
  tradeRing: TradeRing;
  stats: Stats24h | null;
  markPx: number;
  indexPx: number;
  fundingRate: number | null; // fractional, e.g. 0.0001
  nextFundingTs: number; // unix ms of next funding settlement

  setSymbols: (list: SymbolMeta[]) => void;
  setSymbol: (id: number) => void;
  setStats: (s: Stats24h) => void;
  updateMark: (markPx: number, indexPx: number) => void;
  setFundingRate: (rate: number) => void;
  setNextFundingTs: (ts: number) => void;
  updateBbo: (
    bidPx: number,
    bidQty: number,
    askPx: number,
    askQty: number,
    ts: number,
    seq: number,
  ) => void;
  applyL2Snapshot: (
    bids: number[][],
    asks: number[][],
    seq: number,
  ) => void;
  applyL2Delta: (
    side: Side,
    px: number,
    qty: number,
    count: number,
    seq: number,
  ) => void;
  addTrade: (t: RecentTrade) => void;
}

const emptyBook: OrderbookState = {
  bids: [],
  asks: [],
  spread: 0,
  spreadPct: 0,
  midPrice: 0,
  seq: 0,
};

const emptyBbo: BboState = {
  bidPx: 0,
  bidQty: 0,
  askPx: 0,
  askQty: 0,
  ts: 0,
  seq: 0,
};

function toLevels(raw: number[][]): PriceLevel[] {
  let total = 0;
  return raw.map((r) => {
    total += (r[1] ?? 0);
    return {
      price: r[0] ?? 0,
      qty: r[1] ?? 0,
      count: r[2] ?? 0,
      total,
    };
  });
}

function calcSpread(
  bids: PriceLevel[],
  asks: PriceLevel[],
): { spread: number; spreadPct: number; midPrice: number } {
  const bestBid = bids[0]?.price ?? 0;
  const bestAsk = asks[0]?.price ?? 0;
  if (bestBid === 0 || bestAsk === 0) {
    return { spread: 0, spreadPct: 0, midPrice: 0 };
  }
  const spread = bestAsk - bestBid;
  const mid = (bestAsk + bestBid) / 2;
  const pct = mid > 0 ? (spread / mid) * 100 : 0;
  return { spread, spreadPct: pct, midPrice: mid };
}

function applyDelta(
  levels: PriceLevel[],
  px: number,
  qty: number,
  count: number,
  ascending: boolean,
): PriceLevel[] {
  const next = levels.filter((l) => l.price !== px);
  if (qty > 0) {
    next.push({ price: px, qty, count, total: 0 });
  }
  next.sort((a, b) =>
    ascending ? a.price - b.price : b.price - a.price,
  );
  // recalculate cumulative totals
  let total = 0;
  for (const l of next) {
    total += l.qty;
    l.total = total;
  }
  return next.slice(0, 20);
}

// ---------------------------------------------------------------------------
// rAF coalescing: buffer L2 deltas and flush once per animation frame.
// Many deltas arriving in the same 16ms window produce one React re-render.
// ---------------------------------------------------------------------------
interface DeltaEntry {
  side: Side;
  px: number;
  qty: number;
  count: number;
  seq: number;
}

let rafPending = false;
let deltaQueue: DeltaEntry[] = [];

function scheduleFlush(flush: () => void): void {
  if (rafPending) return;
  rafPending = true;
  requestAnimationFrame(() => {
    rafPending = false;
    flush();
  });
}

export const useMarketStore = create<MarketStore>(
  (set) => {
    const flushDeltas = () => {
      const batch = deltaQueue;
      deltaQueue = [];
      if (batch.length === 0) return;
      set((state) => {
        let ob = state.orderbook;
        for (const d of batch) {
          if (d.seq <= ob.seq) continue;
          const bids =
            d.side === Side.BUY
              ? applyDelta(ob.bids, d.px, d.qty, d.count, false)
              : ob.bids;
          const asks =
            d.side === Side.SELL
              ? applyDelta(ob.asks, d.px, d.qty, d.count, true)
              : ob.asks;
          const s = calcSpread(bids, asks);
          ob = { bids, asks, ...s, seq: d.seq };
        }
        return { orderbook: ob };
      });
    };

    return {
      symbols: new Map(),
      selectedSymbol: 1,
      orderbook: emptyBook,
      bbo: emptyBbo,
      tradeRing: new TradeRing(),
      stats: null,
      markPx: 0,
      indexPx: 0,
      fundingRate: null,
      nextFundingTs: 0,

      setSymbols: (list) =>
        set(() => {
          const m = new Map<number, SymbolMeta>();
          for (const s of list) m.set(s.id, s);
          return { symbols: m };
        }),

      setSymbol: (id) => {
        deltaQueue = [];
        set({
          selectedSymbol: id,
          orderbook: emptyBook,
          bbo: emptyBbo,
          tradeRing: new TradeRing(),
          stats: null,
          markPx: 0,
          indexPx: 0,
          fundingRate: null,
          nextFundingTs: 0,
        });
      },

      setStats: (s) => set({ stats: s }),

      updateMark: (markPx, indexPx) =>
        set({ markPx, indexPx }),

      setFundingRate: (rate) =>
        set({ fundingRate: rate }),

      setNextFundingTs: (ts) =>
        set({ nextFundingTs: ts }),

      updateBbo: (bidPx, bidQty, askPx, askQty, ts, seq) =>
        set({ bbo: { bidPx, bidQty, askPx, askQty, ts, seq } }),

      applyL2Snapshot: (rawBids, rawAsks, seq) => {
        deltaQueue = []; // snapshot supersedes any buffered deltas
        set(() => {
          const bids = toLevels(rawBids);
          const asks = toLevels(rawAsks);
          const s = calcSpread(bids, asks);
          return {
            orderbook: { bids, asks, ...s, seq },
          };
        });
      },

      applyL2Delta: (side, px, qty, count, seq) => {
        deltaQueue.push({ side, px, qty, count, seq });
        scheduleFlush(flushDeltas);
      },

      addTrade: (t) =>
        set((state) => ({
          tradeRing: state.tradeRing.push(t),
        })),
    };
  },
);

// --------------- Split selectors with shallow equality ---------------

// Orderbook: re-renders only when orderbook reference changes.
export function useOrderbook(): OrderbookState {
  return useMarketStore(
    useShallow((s) => s.orderbook),
  );
}

// BBO: re-renders only when any BBO field changes.
export function useBbo(): BboState {
  return useMarketStore(
    useShallow((s) => s.bbo),
  );
}

// Trades snapshot (newest-first) from ring buffer.
// Memoized: snapshot() only called when tradeRing reference changes.
export function useTrades(): RecentTrade[] {
  const ring = useMarketStore((s) => s.tradeRing);
  return useMemo(() => ring.snapshot(), [ring]);
}

// Symbol metadata for the currently selected symbol.
export function useSymbolMeta(): SymbolMeta | undefined {
  return useMarketStore(
    useShallow((s) => s.symbols.get(s.selectedSymbol)),
  );
}

// 24h stats for the selected symbol (null until received).
// Shallow-compared: no re-render if stats fields are identical.
export function useStats(): Stats24h | null {
  return useMarketStore(useShallow((s) => s.stats));
}

export interface FundingData {
  markPx: number;
  indexPx: number;
  fundingRate: number | null;
  nextFundingTs: number;
}

// Mark/index/funding data (shallow-compared).
export function useFundingData(): FundingData {
  return useMarketStore(
    useShallow((s) => ({
      markPx: s.markPx,
      indexPx: s.indexPx,
      fundingRate: s.fundingRate,
      nextFundingTs: s.nextFundingTs,
    })),
  );
}
