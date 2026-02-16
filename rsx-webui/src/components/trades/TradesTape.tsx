import clsx from "clsx";
import { useRef } from "react";
import { useEffect } from "react";
import { useMarketStore } from "../../store/market";
import { formatPrice } from "../../lib/format";
import { formatQty } from "../../lib/format";
import { formatTs } from "../../lib/format";
import { Side } from "../../lib/protocol";

export function TradesTape() {
  const trades = useMarketStore((s) => s.trades);
  const symbols = useMarketStore((s) => s.symbols);
  const selectedSymbol = useMarketStore(
    (s) => s.selectedSymbol,
  );
  const scrollRef = useRef<HTMLDivElement>(null);
  const meta = symbols.get(selectedSymbol);
  const tickSize = meta?.tickSize ?? 0.01;
  const lotSize = meta?.lotSize ?? 0.001;

  // Auto-scroll to top (newest first).
  // Use first trade's ts as trigger so scroll fires
  // even when array is capped at 100 entries.
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
          <div
            key={`${t.ts}-${i}`}
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
        ))}
      </div>
    </div>
  );
}
