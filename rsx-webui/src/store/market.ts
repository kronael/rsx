import { create } from "zustand";
import type {
  SymbolMeta,
  OrderbookState,
  BboState,
  RecentTrade,
  PriceLevel,
} from "../lib/types";
import { Side } from "../lib/protocol";

interface MarketStore {
  symbols: Map<number, SymbolMeta>;
  selectedSymbol: number;
  orderbook: OrderbookState;
  bbo: BboState;
  trades: RecentTrade[];

  setSymbols: (list: SymbolMeta[]) => void;
  setSymbol: (id: number) => void;
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

export const useMarketStore = create<MarketStore>(
  (set) => ({
    symbols: new Map(),
    selectedSymbol: 1,
    orderbook: emptyBook,
    bbo: emptyBbo,
    trades: [],

    setSymbols: (list) =>
      set(() => {
        const m = new Map<number, SymbolMeta>();
        for (const s of list) m.set(s.id, s);
        return { symbols: m };
      }),

    setSymbol: (id) =>
      set({
        selectedSymbol: id,
        orderbook: emptyBook,
        bbo: emptyBbo,
        trades: [],
      }),

    updateBbo: (bidPx, bidQty, askPx, askQty, ts, seq) =>
      set({ bbo: { bidPx, bidQty, askPx, askQty, ts, seq } }),

    applyL2Snapshot: (rawBids, rawAsks, seq) =>
      set(() => {
        const bids = toLevels(rawBids);
        const asks = toLevels(rawAsks);
        const s = calcSpread(bids, asks);
        return {
          orderbook: { bids, asks, ...s, seq },
        };
      }),

    applyL2Delta: (side, px, qty, count, seq) =>
      set((state) => {
        const ob = state.orderbook;
        if (seq <= ob.seq) return state;
        const bids =
          side === Side.BUY
            ? applyDelta(ob.bids, px, qty, count, false)
            : ob.bids;
        const asks =
          side === Side.SELL
            ? applyDelta(ob.asks, px, qty, count, true)
            : ob.asks;
        const s = calcSpread(bids, asks);
        return {
          orderbook: { bids, asks, ...s, seq },
        };
      }),

    addTrade: (t) =>
      set((state) => ({
        trades: [t, ...state.trades].slice(0, 100),
      })),
  }),
);
