import { useMemo } from "react";
import clsx from "clsx";
import { useMarketStore } from "../../store/market";
import { formatPrice } from "../../lib/format";
import { formatQty } from "../../lib/format";
import type { PriceLevel } from "../../lib/types";

function Row({
  level,
  maxTotal,
  isBid,
  tickSize,
  lotSize,
  onClick,
}: {
  level: PriceLevel;
  maxTotal: number;
  isBid: boolean;
  tickSize: number;
  lotSize: number;
  onClick?: (price: number) => void;
}) {
  const pct =
    maxTotal > 0
      ? (level.total / maxTotal) * 100
      : 0;

  return (
    <div
      className="relative flex items-center px-2
        py-[1px] text-xs font-mono cursor-pointer
        hover:bg-bg-hover"
      role="button"
      tabIndex={0}
      onClick={() => onClick?.(level.price)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onClick?.(level.price);
        }
      }}
    >
      {/* Depth bar background */}
      <div
        className={clsx(
          "absolute inset-y-0 right-0",
          isBid ? "bg-buy/10" : "bg-sell/10",
        )}
        style={{ width: `${pct}%` }}
      />
      <span
        className={clsx(
          "w-[80px] text-right z-10",
          isBid ? "text-buy" : "text-sell",
        )}
      >
        {formatPrice(level.price, tickSize)}
      </span>
      <span className="w-[70px] text-right z-10
        text-text-primary"
      >
        {formatQty(level.qty, lotSize)}
      </span>
      <span className="flex-1 text-right z-10
        text-text-secondary"
      >
        {formatQty(level.total, lotSize)}
      </span>
    </div>
  );
}

interface OrderbookProps {
  onPriceClick?: (price: number) => void;
}

export function Orderbook({ onPriceClick }: OrderbookProps) {
  const orderbook = useMarketStore((s) => s.orderbook);
  const symbols = useMarketStore((s) => s.symbols);
  const selectedSymbol = useMarketStore(
    (s) => s.selectedSymbol,
  );
  const meta = symbols.get(selectedSymbol);
  const tickSize = meta?.tickSize ?? 0.01;
  const lotSize = meta?.lotSize ?? 0.001;

  const asks = orderbook.asks.slice(0, 10);
  const bids = orderbook.bids.slice(0, 10);

  const asksReversed = useMemo(
    () => [...asks].reverse(),
    [asks],
  );

  const maxAskTotal = asks.length > 0
    ? asks[asks.length - 1]?.total ?? 0
    : 0;
  const maxBidTotal = bids.length > 0
    ? bids[bids.length - 1]?.total ?? 0
    : 0;

  const spreadPct = Number.isFinite(orderbook.spreadPct)
    ? orderbook.spreadPct.toFixed(2)
    : "0.00";
  const spreadStr = orderbook.spread > 0
    ? `${formatPrice(orderbook.spread, tickSize)} (${spreadPct}%)`
    : "--";

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex px-2 py-1 text-2xs
        text-text-secondary border-b border-border"
      >
        <span className="w-[80px] text-right">Price</span>
        <span className="w-[70px] text-right">Size</span>
        <span className="flex-1 text-right">Total</span>
      </div>

      {/* Asks (reversed so lowest ask is at bottom) */}
      <div className="flex-1 flex flex-col justify-end
        overflow-hidden"
      >
        {asksReversed.map((level) => (
          <Row
            key={level.price}
            level={level}
            maxTotal={maxAskTotal}
            isBid={false}
            tickSize={tickSize}
            lotSize={lotSize}
            onClick={onPriceClick}
          />
        ))}
      </div>

      {/* Spread bar */}
      <div className="flex items-center justify-center
        px-2 py-1 text-xs text-text-secondary
        border-y border-border bg-bg-surface"
      >
        Spread: {spreadStr}
      </div>

      {/* Bids */}
      <div className="flex-1 overflow-hidden">
        {bids.map((level) => (
          <Row
            key={level.price}
            level={level}
            maxTotal={maxBidTotal}
            isBid={true}
            tickSize={tickSize}
            lotSize={lotSize}
            onClick={onPriceClick}
          />
        ))}
      </div>
    </div>
  );
}
