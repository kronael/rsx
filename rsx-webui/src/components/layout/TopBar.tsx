import { useEffect, useState, useRef, useMemo } from "react";
import clsx from "clsx";
import { useMarketStore } from "../../store/market";
import { useBbo } from "../../store/market";
import { useSymbolMeta } from "../../store/market";
import { useStats } from "../../store/market";
import { useFundingData } from "../../store/market";
import { useConnectionStore } from "../../store/connection";
import { useTradingStore } from "../../store/trading";
import { formatPrice } from "../../lib/format";
import { formatQty } from "../../lib/format";
import { WsStatus } from "../../lib/types";
import { fetchSymbols } from "../../hooks/useRestApi";
import { fetchAccount } from "../../hooks/useRestApi";
import { fetchPositions } from "../../hooks/useRestApi";
import { fetchOrders } from "../../hooks/useRestApi";

// ▲ up, ▼ down, — flat
type TickDir = "up" | "down" | "flat";

function TickArrow({ dir }: { dir: TickDir }) {
  if (dir === "up") {
    return (
      <span className="text-buy text-[10px] leading-none">
        ▲
      </span>
    );
  }
  if (dir === "down") {
    return (
      <span className="text-sell text-[10px] leading-none">
        ▼
      </span>
    );
  }
  return null;
}

interface StatCellProps {
  label: string;
  value: string;
  valueClass?: string;
}

function StatCell({ label, value, valueClass }: StatCellProps) {
  return (
    <div className="flex flex-col min-w-[70px]">
      <span className="text-text-secondary text-2xs">
        {label}
      </span>
      <span
        className={clsx(
          "font-mono text-xs text-text-primary",
          valueClass,
        )}
      >
        {value}
      </span>
    </div>
  );
}

export function TopBar() {
  const [dropdownOpen, setDropdownOpen] = useState(false);
  const dropRef = useRef<HTMLDivElement>(null);
  const prevBidRef = useRef<number>(0);
  const [tickDir, setTickDir] = useState<TickDir>("flat");
  const [countdown, setCountdown] = useState("");

  const symbols = useMarketStore((s) => s.symbols);
  const selectedSymbol = useMarketStore(
    (s) => s.selectedSymbol,
  );
  const setSymbol = useMarketStore((s) => s.setSymbol);
  const setSymbols = useMarketStore((s) => s.setSymbols);
  const bbo = useBbo();
  const meta = useSymbolMeta();
  const stats = useStats();
  const funding = useFundingData();
  const privStatus = useConnectionStore(
    (s) => s.privateStatus,
  );
  const pubStatus = useConnectionStore(
    (s) => s.publicStatus,
  );
  const latency = useConnectionStore((s) => s.latency);

  const tickSize = meta?.tickSize ?? 0.01;
  const lotSize = meta?.lotSize ?? 0.001;

  // Track tick direction from BBO bid price changes
  useEffect(() => {
    const cur = bbo.bidPx;
    const prev = prevBidRef.current;
    if (prev !== 0 && cur !== prev) {
      setTickDir(cur > prev ? "up" : "down");
    }
    prevBidRef.current = cur;
  }, [bbo.bidPx]);

  // Funding countdown timer: ticks every second.
  // If nextFundingTs is not set, compute next 8h UTC boundary.
  useEffect(() => {
    function calc(): string {
      const target =
        funding.nextFundingTs > 0
          ? funding.nextFundingTs
          : (() => {
              const now = Date.now();
              const interval = 8 * 60 * 60 * 1000;
              return (
                Math.ceil(now / interval) * interval
              );
            })();
      const diff = Math.max(0, target - Date.now());
      const h = Math.floor(diff / 3_600_000);
      const m = Math.floor((diff % 3_600_000) / 60_000);
      const s = Math.floor((diff % 60_000) / 1_000);
      return [
        h.toString().padStart(2, "0"),
        m.toString().padStart(2, "0"),
        s.toString().padStart(2, "0"),
      ].join(":");
    }
    setCountdown(calc());
    const id = setInterval(() => setCountdown(calc()), 1000);
    return () => clearInterval(id);
  }, [funding.nextFundingTs]);

  // Load symbols + initial data on mount
  useEffect(() => {
    fetchSymbols()
      .then((list) => {
        setSymbols(list);
      })
      .catch(() => {});
    fetchAccount()
      .then((a) => useTradingStore.getState().setAccount(a))
      .catch(() => {});
    fetchPositions()
      .then((p) =>
        useTradingStore.getState().setPositions(p),
      )
      .catch(() => {});
    fetchOrders()
      .then((o) => useTradingStore.getState().setOrders(o))
      .catch(() => {});
  }, [setSymbols]);

  // Close dropdown on outside click or Escape
  useEffect(() => {
    function handleClick(e: MouseEvent) {
      if (
        dropRef.current &&
        !dropRef.current.contains(e.target as Node)
      ) {
        setDropdownOpen(false);
      }
    }
    function handleKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        setDropdownOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClick);
    document.addEventListener("keydown", handleKey);
    return () => {
      document.removeEventListener(
        "mousedown", handleClick,
      );
      document.removeEventListener(
        "keydown", handleKey,
      );
    };
  }, []);

  const statusColor =
    privStatus === WsStatus.CONNECTED &&
    pubStatus === WsStatus.CONNECTED
      ? "bg-buy"
      : privStatus === WsStatus.CONNECTING ||
          pubStatus === WsStatus.CONNECTING
        ? "bg-accent"
        : "bg-sell";

  const lastPx = bbo.bidPx > 0
    ? formatPrice(bbo.bidPx, tickSize)
    : "--";

  const lastPxColor =
    tickDir === "up"
      ? "text-buy"
      : tickDir === "down"
        ? "text-sell"
        : "text-text-primary";

  // 24h derived stats
  const { changePct, changePctStr, high24h, low24h, vol24h } =
    useMemo(() => {
      if (!stats || stats.open === 0) {
        return {
          changePct: 0,
          changePctStr: "--",
          high24h: "--",
          low24h: "--",
          vol24h: "--",
        };
      }
      const pct =
        ((stats.close - stats.open) / stats.open) * 100;
      return {
        changePct: pct,
        changePctStr:
          (pct >= 0 ? "+" : "") + pct.toFixed(2) + "%",
        high24h: formatPrice(stats.high, tickSize),
        low24h: formatPrice(stats.low, tickSize),
        vol24h: formatQty(stats.volume, lotSize),
      };
    }, [stats, tickSize, lotSize]);

  const changeColor =
    changePct > 0
      ? "text-buy"
      : changePct < 0
        ? "text-sell"
        : "text-text-secondary";

  const markPxStr =
    funding.markPx > 0
      ? formatPrice(funding.markPx, tickSize)
      : "--";
  const indexPxStr =
    funding.indexPx > 0
      ? formatPrice(funding.indexPx, tickSize)
      : "--";
  const fundingRateStr =
    funding.fundingRate !== null
      ? (funding.fundingRate >= 0 ? "+" : "") +
        (funding.fundingRate * 100).toFixed(4) +
        "%"
      : "--";
  const fundingRateColor =
    funding.fundingRate === null
      ? "text-text-secondary"
      : funding.fundingRate > 0
        ? "text-buy"
        : funding.fundingRate < 0
          ? "text-sell"
          : "text-text-secondary";

  return (
    <div
      className="h-12 bg-bg-surface border-b border-border
        flex items-center px-4 gap-4 shrink-0 overflow-x-auto"
    >
      {/* Symbol selector */}
      <div className="relative shrink-0" ref={dropRef}>
        <button
          className="text-text-primary font-semibold
            text-sm hover:text-accent transition-colors"
          onClick={() => setDropdownOpen(!dropdownOpen)}
          aria-expanded={dropdownOpen}
          aria-haspopup="listbox"
          aria-label="Select trading pair"
        >
          {meta?.name ?? "Loading..."} &#9662;
        </button>
        {dropdownOpen && (
          <div
            role="listbox"
            className="absolute top-10 left-0 z-50
              bg-bg-surface border border-border rounded
              shadow-lg min-w-[160px]"
          >
            {Array.from(symbols.values()).map((s) => (
              <button
                key={s.id}
                role="option"
                aria-selected={s.id === selectedSymbol}
                className={clsx(
                  "block w-full text-left px-3 py-2",
                  "text-sm hover:bg-bg-hover",
                  s.id === selectedSymbol
                    ? "text-accent"
                    : "text-text-primary",
                )}
                onClick={() => {
                  setSymbol(s.id);
                  setDropdownOpen(false);
                }}
              >
                {s.name}
              </button>
            ))}
          </div>
        )}
      </div>

      {/* Last price + tick arrow */}
      <div className="flex items-center gap-1 shrink-0">
        <span
          className={clsx(
            "font-mono text-sm font-semibold",
            lastPxColor,
          )}
        >
          {lastPx}
        </span>
        <TickArrow dir={tickDir} />
      </div>

      {/* 24h change */}
      <StatCell
        label="24h Change"
        value={changePctStr}
        valueClass={changeColor}
      />

      {/* 24h High */}
      <StatCell label="24h High" value={high24h} />

      {/* 24h Low */}
      <StatCell label="24h Low" value={low24h} />

      {/* 24h Volume */}
      <StatCell label="24h Vol" value={vol24h} />

      {/* BBO */}
      <div className="flex items-center gap-3 text-xs shrink-0">
        <div className="flex flex-col">
          <span className="text-text-secondary text-2xs">
            Bid
          </span>
          <span className="font-mono text-buy">
            {bbo.bidPx > 0
              ? formatPrice(bbo.bidPx, tickSize)
              : "--"}
          </span>
        </div>
        <div className="flex flex-col">
          <span className="text-text-secondary text-2xs">
            Ask
          </span>
          <span className="font-mono text-sell">
            {bbo.askPx > 0
              ? formatPrice(bbo.askPx, tickSize)
              : "--"}
          </span>
        </div>
      </div>

      {/* Mark price */}
      <StatCell label="Mark" value={markPxStr} />

      {/* Index price */}
      <StatCell label="Index" value={indexPxStr} />

      {/* Funding rate + countdown */}
      <div className="flex flex-col min-w-[90px] shrink-0">
        <span className="text-text-secondary text-2xs">
          Funding / Countdown
        </span>
        <div className="flex items-center gap-1">
          <span
            className={clsx(
              "font-mono text-xs",
              fundingRateColor,
            )}
          >
            {fundingRateStr}
          </span>
          <span className="text-text-secondary text-2xs">
            {countdown}
          </span>
        </div>
      </div>

      {/* Spacer */}
      <div className="flex-1" />

      {/* Connection status */}
      <div
        className="flex items-center gap-2 text-xs shrink-0"
        aria-label="Connection status"
      >
        <div
          className={clsx(
            "w-2 h-2 rounded-full",
            statusColor,
          )}
          role="status"
          aria-label={
            privStatus === WsStatus.CONNECTED &&
            pubStatus === WsStatus.CONNECTED
              ? "Connected"
              : "Disconnected"
          }
        />
        <span className="text-text-secondary font-mono">
          {latency > 0 ? `${latency}ms` : "--"}
        </span>
      </div>
    </div>
  );
}
