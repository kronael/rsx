import { memo } from "react";
import { useRef } from "react";
import { useEffect } from "react";
import { useState } from "react";
import clsx from "clsx";
import { useMarketStore } from "../../store/market";
import { useSymbolMeta } from "../../store/market";
import type { TradeRing } from "../../store/market";
import { formatPrice } from "../../lib/format";
import { formatQty } from "../../lib/format";
import { formatTs } from "../../lib/format";
import { Side } from "../../lib/protocol";
import type { RecentTrade } from "../../lib/types";

// Aggregated trade row (may represent multiple fills at same price)
interface AggTrade {
  price: number;
  qty: number;       // sum of qty in bucket
  side: Side;
  ts: number;        // timestamp of latest fill
  seq: number;
  count: number;     // number of fills aggregated
  key: string;       // stable react key
}

// Bucket consecutive trades at the same price+side together.
function aggregate(trades: RecentTrade[]): AggTrade[] {
  const out: AggTrade[] = [];
  for (const t of trades) {
    const last = out[out.length - 1];
    if (
      last &&
      last.price === t.price &&
      last.side === t.side
    ) {
      last.qty += t.qty;
      last.ts = t.ts;
      last.count++;
    } else {
      out.push({
        price: t.price,
        qty: t.qty,
        side: t.side,
        ts: t.ts,
        seq: t.seq,
        count: 1,
        key: `${t.price}-${t.ts}-${t.seq}`,
      });
    }
  }
  return out;
}

// Large-trade threshold: top 5% by qty in current view.
function largeThreshold(trades: AggTrade[]): number {
  if (trades.length === 0) return Infinity;
  const sorted = trades.map((t) => t.qty).sort((a, b) => b - a);
  const idx = Math.max(
    0,
    Math.floor(sorted.length * 0.05) - 1,
  );
  return sorted[idx] ?? Infinity;
}

const TradeRow = memo(function TradeRow({
  t,
  tickSize,
  lotSize,
  maxQty,
  isLarge,
  flash,
}: {
  t: AggTrade;
  tickSize: number;
  lotSize: number;
  maxQty: number;
  isLarge: boolean;
  flash: boolean;
}) {
  const isBuy = t.side === Side.BUY;
  const barPct = maxQty > 0 ? (t.qty / maxQty) * 100 : 0;

  return (
    <div
      className={clsx(
        "relative flex items-center px-2 py-[1px]",
        "text-xs font-mono",
        flash && (isBuy
          ? "animate-flash-buy"
          : "animate-flash-sell"),
        isLarge && "font-semibold",
      )}
    >
      {/* Size bar */}
      <div
        className={clsx(
          "absolute inset-y-0 left-0",
          isBuy ? "bg-buy/8" : "bg-sell/8",
        )}
        style={{ width: `${barPct}%` }}
      />
      <span
        className={clsx(
          "w-[80px] text-right z-10",
          isBuy ? "text-buy" : "text-sell",
          isLarge && "brightness-125",
        )}
      >
        {formatPrice(t.price, tickSize)}
      </span>
      <span
        className={clsx(
          "w-[60px] text-right z-10",
          isLarge
            ? (isBuy ? "text-buy" : "text-sell")
            : "text-text-primary",
        )}
      >
        {formatQty(t.qty, lotSize)}
        {t.count > 1 && (
          <span className="text-text-secondary text-2xs ml-0.5">
            ×{t.count}
          </span>
        )}
      </span>
      <span className="flex-1 text-right z-10
        text-text-secondary"
      >
        {formatTs(t.ts)}
      </span>
    </div>
  );
});

// Subscribe to ring buffer; coalesce updates via rAF so
// the component re-renders at most once per animation frame.
function useThrottledTrades(): RecentTrade[] {
  const [trades, setTrades] = useState<RecentTrade[]>(() =>
    useMarketStore.getState().tradeRing.snapshot(),
  );
  const rafRef = useRef<number | null>(null);
  const pendingRef = useRef<TradeRing | null>(null);

  useEffect(() => {
    const unsub = useMarketStore.subscribe((state) => {
      pendingRef.current = state.tradeRing;
      if (rafRef.current !== null) return;
      rafRef.current = requestAnimationFrame(() => {
        rafRef.current = null;
        const ring = pendingRef.current;
        if (ring !== null) {
          pendingRef.current = null;
          setTrades(ring.snapshot());
        }
      });
    });
    return () => {
      unsub();
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
    };
  }, []);

  return trades;
}

export function TradesTape() {
  const rawTrades = useThrottledTrades();
  const meta = useSymbolMeta();
  const scrollRef = useRef<HTMLDivElement>(null);
  const tickSize = meta?.tickSize ?? 0.01;
  const lotSize = meta?.lotSize ?? 0.001;

  const [aggMode, setAggMode] = useState(false);
  // Track the newest seq we have seen to determine flash rows
  const prevNewestSeq = useRef<number>(-1);

  const trades = aggMode
    ? aggregate(rawTrades)
    : rawTrades.map((t, i): AggTrade => ({
        ...t,
        count: 1,
        key: `${t.ts}-${i}`,
      }));

  const maxQty = trades.reduce(
    (m, t) => (t.qty > m ? t.qty : m),
    0,
  );
  const largeThr = largeThreshold(trades);

  // Determine how many rows at the top are new this frame
  const newestSeq = rawTrades[0]?.seq ?? -1;
  let newCount = 0;
  if (newestSeq !== prevNewestSeq.current) {
    // Count raw trades newer than previous newest
    for (const t of rawTrades) {
      if (t.seq > prevNewestSeq.current) newCount++;
      else break;
    }
  }
  // Update ref after computing newCount
  useEffect(() => {
    prevNewestSeq.current = newestSeq;
  }, [newestSeq]);

  // Auto-scroll to top (newest first).
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = 0;
    }
  }, [newestSeq]);

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center px-2 py-1
        text-2xs text-text-secondary border-b border-border"
      >
        <span className="w-[80px] text-right">Price</span>
        <span className="w-[60px] text-right">Size</span>
        <span className="flex-1 text-right">Time</span>
        <button
          className={clsx(
            "ml-2 px-1.5 py-0.5 rounded text-2xs",
            aggMode
              ? "bg-bg-hover text-text-primary"
              : "text-text-secondary hover:text-text-primary",
          )}
          onClick={() => setAggMode((v) => !v)}
          title="Aggregate consecutive fills at same price"
          aria-pressed={aggMode}
        >
          Agg
        </button>
      </div>
      <div
        ref={scrollRef}
        className="flex-1 overflow-y-auto"
      >
        {trades.length === 0 && (
          <div className="flex items-center justify-center
            h-full text-text-secondary text-xs"
          >
            No recent trades
          </div>
        )}
        {trades.map((t, i) => (
          <TradeRow
            key={t.key}
            t={t}
            tickSize={tickSize}
            lotSize={lotSize}
            maxQty={maxQty}
            isLarge={t.qty >= largeThr}
            flash={i < newCount}
          />
        ))}
      </div>
    </div>
  );
}
