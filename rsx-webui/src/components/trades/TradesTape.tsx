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

const TradeRow = memo(function TradeRow({
  t,
  tickSize,
  lotSize,
}: {
  t: RecentTrade;
  tickSize: number;
  lotSize: number;
}) {
  return (
    <div
      className="flex items-center px-2 py-[1px]
        text-xs font-mono"
    >
      <span
        className={clsx(
          "w-[80px] text-right",
          t.side === Side.BUY
            ? "text-buy"
            : "text-sell",
        )}
      >
        {formatPrice(t.price, tickSize)}
      </span>
      <span className="w-[60px] text-right
        text-text-primary"
      >
        {formatQty(t.qty, lotSize)}
      </span>
      <span className="flex-1 text-right
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
  const trades = useThrottledTrades();
  const meta = useSymbolMeta();
  const scrollRef = useRef<HTMLDivElement>(null);
  const tickSize = meta?.tickSize ?? 0.01;
  const lotSize = meta?.lotSize ?? 0.001;

  // Auto-scroll to top (newest first).
  const newestTs = trades[0]?.ts ?? 0;
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = 0;
    }
  }, [newestTs]);

  return (
    <div className="flex flex-col h-full">
      <div className="flex px-2 py-1 text-2xs
        text-text-secondary border-b border-border"
      >
        <span className="w-[80px] text-right">Price</span>
        <span className="w-[60px] text-right">Size</span>
        <span className="flex-1 text-right">Time</span>
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
            key={`${t.ts}-${i}`}
            t={t}
            tickSize={tickSize}
            lotSize={lotSize}
          />
        ))}
      </div>
    </div>
  );
}
