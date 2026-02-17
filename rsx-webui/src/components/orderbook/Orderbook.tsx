import {
  memo,
  useMemo,
  useCallback,
  useRef,
  useEffect,
  useState,
} from "react";
import clsx from "clsx";
import { useOrderbook } from "../../store/market";
import { useSymbolMeta } from "../../store/market";
import { useMarketStore } from "../../store/market";
import { formatPrice } from "../../lib/format";
import { formatQty } from "../../lib/format";
import type { PriceLevel } from "../../lib/types";
import { Side } from "../../lib/protocol";

type SideView = "both" | "bids" | "asks";

const TICK_MULTIPLES = [1, 2, 5, 10, 25, 50] as const;

// Group levels into coarser tick buckets, summing qty/count.
function groupLevels(
  levels: PriceLevel[],
  mult: number,
  ascending: boolean,
): PriceLevel[] {
  if (mult === 1) return levels;
  const buckets = new Map<number, PriceLevel>();
  for (const l of levels) {
    const bucket = ascending
      ? Math.floor(l.price / mult) * mult
      : Math.ceil(l.price / mult) * mult;
    const existing = buckets.get(bucket);
    if (existing) {
      existing.qty += l.qty;
      existing.count += l.count;
    } else {
      buckets.set(bucket, {
        price: bucket,
        qty: l.qty,
        count: l.count,
        total: 0,
      });
    }
  }
  const sorted = Array.from(buckets.values()).sort(
    (a, b) => ascending
      ? a.price - b.price
      : b.price - a.price,
  );
  let total = 0;
  for (const l of sorted) {
    total += l.qty;
    l.total = total;
  }
  return sorted.slice(0, 20);
}

const Row = memo(function Row({
  level,
  maxTotal,
  isBid,
  showCount,
  tickSize,
  lotSize,
  onClick,
}: {
  level: PriceLevel;
  maxTotal: number;
  isBid: boolean;
  showCount: boolean;
  tickSize: number;
  lotSize: number;
  onClick?: (price: number) => void;
}) {
  const pct =
    maxTotal > 0
      ? (level.total / maxTotal) * 100
      : 0;

  // Flash row background when qty changes
  const prevQty = useRef(level.qty);
  const rowRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (prevQty.current !== level.qty && rowRef.current) {
      const el = rowRef.current;
      const cls = isBid ? "animate-flash-buy" : "animate-flash-sell";
      el.classList.remove("animate-flash-buy", "animate-flash-sell");
      void el.offsetWidth; // force reflow to restart animation
      el.classList.add(cls);
    }
    prevQty.current = level.qty;
  }, [level.qty, isBid]);

  const handleClick = useCallback(() => {
    onClick?.(level.price);
  }, [onClick, level.price]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        onClick?.(level.price);
      }
    },
    [onClick, level.price],
  );

  return (
    <div
      ref={rowRef}
      className="relative flex items-center px-2
        py-[1px] text-xs font-mono cursor-pointer
        hover:bg-bg-hover"
      role="button"
      tabIndex={0}
      onClick={handleClick}
      onKeyDown={handleKeyDown}
    >
      {/* Depth bar background (full-width, scaleX from right) */}
      <div
        className={clsx(
          "absolute inset-y-0 right-0 w-full origin-right",
          isBid ? "bg-buy/10" : "bg-sell/10",
        )}
        style={{
          transform: `scaleX(${pct / 100})`,
          willChange: "transform",
        }}
      />
      {/* Edge accent bar — right edge for bids, left edge for asks */}
      {pct > 0 && (
        <div
          className={clsx(
            "absolute inset-y-0 w-[2px]",
            isBid ? "right-0 bg-buy/70" : "left-0 bg-sell/70",
          )}
        />
      )}
      <span
        className={clsx(
          "w-[80px] text-right z-10",
          isBid ? "text-buy" : "text-sell",
        )}
      >
        {formatPrice(level.price, tickSize)}
      </span>
      <span className="w-[60px] text-right z-10
        text-text-primary"
      >
        {formatQty(level.qty, lotSize)}
      </span>
      {showCount && (
        <span className="w-[36px] text-right z-10
          text-text-secondary"
        >
          {level.count}
        </span>
      )}
      <span className="flex-1 text-right z-10
        text-text-secondary"
      >
        {formatQty(level.total, lotSize)}
      </span>
    </div>
  );
});

interface OrderbookProps {
  onPriceClick?: (price: number) => void;
}

export function Orderbook({ onPriceClick }: OrderbookProps) {
  const orderbook = useOrderbook();
  const meta = useSymbolMeta();
  const tickSize = meta?.tickSize ?? 0.01;
  const lotSize = meta?.lotSize ?? 0.001;

  const lastTrade = useMarketStore(
    (s) => s.tradeRing.newest(),
  );

  const [sideView, setSideView] = useState<SideView>("both");
  const [tickMult, setTickMult] = useState(1);
  const [showCount, setShowCount] = useState(false);

  const {
    bids,
    asksReversed,
    maxAskTotal,
    maxBidTotal,
  } = useMemo(() => {
    const limit = sideView === "both" ? 10 : 20;
    const rawAsks = orderbook.asks.slice(0, 20);
    const rawBids = orderbook.bids.slice(0, 20);
    const a = groupLevels(rawAsks, tickMult, true)
      .slice(0, limit);
    const b = groupLevels(rawBids, tickMult, false)
      .slice(0, limit);
    return {
      bids: b,
      asksReversed: [...a].reverse(),
      maxAskTotal: a.length > 0
        ? (a[a.length - 1]?.total ?? 0)
        : 0,
      maxBidTotal: b.length > 0
        ? (b[b.length - 1]?.total ?? 0)
        : 0,
    };
  }, [orderbook.asks, orderbook.bids, tickMult, sideView]);

  const spreadStr = useMemo(() => {
    if (orderbook.spread <= 0) return "--";
    const pct = Number.isFinite(orderbook.spreadPct)
      ? orderbook.spreadPct.toFixed(2)
      : "0.00";
    return `${formatPrice(
      orderbook.spread, tickSize,
    )} (${pct}%)`;
  }, [orderbook.spread, orderbook.spreadPct, tickSize]);

  const lastPriceStr = lastTrade
    ? formatPrice(lastTrade.price, tickSize)
    : "--";
  const isBuy = lastTrade?.side === Side.BUY;
  const isSell = lastTrade?.side === Side.SELL;

  const showAsks = sideView !== "bids";
  const showBids = sideView !== "asks";

  return (
    <div className="flex flex-col h-full">
      {/* Controls */}
      <div className="flex items-center gap-1 px-2 py-1
        border-b border-border shrink-0"
      >
        {/* Side toggle */}
        <div className="flex gap-0.5">
          {(["both", "bids", "asks"] as SideView[]).map(
            (v) => (
              <button
                key={v}
                className={clsx(
                  "px-1.5 py-0.5 text-2xs rounded capitalize",
                  sideView === v
                    ? "bg-bg-hover text-text-primary"
                    : "text-text-secondary"
                      + " hover:text-text-primary",
                )}
                onClick={() => setSideView(v)}
                aria-pressed={sideView === v}
              >
                {v}
              </button>
            ),
          )}
        </div>

        {/* Tick grouping */}
        <select
          className="ml-1 text-2xs bg-bg-surface
            border border-border rounded px-1 py-0.5
            text-text-secondary focus:outline-none
            focus:border-accent"
          value={tickMult}
          onChange={(e) =>
            setTickMult(Number(e.target.value))
          }
          aria-label="Tick grouping"
        >
          {TICK_MULTIPLES.map((m) => (
            <option key={m} value={m}>
              {m === 1 ? "1x" : `${m}x`}
            </option>
          ))}
        </select>

        {/* Count column toggle */}
        <button
          className={clsx(
            "ml-auto px-1.5 py-0.5 text-2xs rounded",
            showCount
              ? "bg-bg-hover text-text-primary"
              : "text-text-secondary"
                + " hover:text-text-primary",
          )}
          onClick={() => setShowCount((v) => !v)}
          aria-pressed={showCount}
          title="Toggle order count column"
        >
          #
        </button>
      </div>

      {/* Column header */}
      <div className="flex px-2 py-1 text-2xs
        text-text-secondary border-b border-border shrink-0"
      >
        <span className="w-[80px] text-right">Price</span>
        <span className="w-[60px] text-right">Size</span>
        {showCount && (
          <span className="w-[36px] text-right">#</span>
        )}
        <span className="flex-1 text-right">Total</span>
      </div>

      {/* Asks (reversed: lowest ask at bottom) */}
      {showAsks && (
        <div className="flex-1 flex flex-col justify-end
          overflow-hidden"
        >
          {asksReversed.map((level) => (
            <Row
              key={level.price}
              level={level}
              maxTotal={maxAskTotal}
              isBid={false}
              showCount={showCount}
              tickSize={tickSize}
              lotSize={lotSize}
              onClick={onPriceClick}
            />
          ))}
        </div>
      )}

      {/* Last price + spread */}
      <div className="flex items-center justify-between
        px-2 py-1 text-xs border-y border-border
        bg-bg-surface shrink-0"
      >
        <span
          className={clsx(
            "flex items-center gap-1 font-mono font-semibold",
            isBuy && "text-buy",
            isSell && "text-sell",
            !isBuy && !isSell && "text-text-primary",
          )}
        >
          {isBuy && <span aria-hidden="true">▲</span>}
          {isSell && <span aria-hidden="true">▼</span>}
          {lastPriceStr}
        </span>
        <span className="text-text-secondary text-2xs">
          {spreadStr}
        </span>
      </div>

      {/* Bids */}
      {showBids && (
        <div className="flex-1 overflow-hidden">
          {bids.map((level) => (
            <Row
              key={level.price}
              level={level}
              maxTotal={maxBidTotal}
              isBid={true}
              showCount={showCount}
              tickSize={tickSize}
              lotSize={lotSize}
              onClick={onPriceClick}
            />
          ))}
        </div>
      )}
    </div>
  );
}
