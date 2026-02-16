import { useEffect } from "react";
import { useState } from "react";
import { useRef } from "react";
import clsx from "clsx";
import { useMarketStore } from "../../store/market";
import { useConnectionStore } from "../../store/connection";
import { useTradingStore } from "../../store/trading";
import { formatPrice } from "../../lib/format";
import { formatQty } from "../../lib/format";
import { WsStatus } from "../../lib/types";
import { fetchSymbols } from "../../hooks/useRestApi";
import { fetchAccount } from "../../hooks/useRestApi";
import { fetchPositions } from "../../hooks/useRestApi";
import { fetchOrders } from "../../hooks/useRestApi";

export function TopBar() {
  const [dropdownOpen, setDropdownOpen] = useState(false);
  const dropRef = useRef<HTMLDivElement>(null);
  const symbols = useMarketStore((s) => s.symbols);
  const selectedSymbol = useMarketStore(
    (s) => s.selectedSymbol,
  );
  const setSymbol = useMarketStore((s) => s.setSymbol);
  const setSymbols = useMarketStore((s) => s.setSymbols);
  const bbo = useMarketStore((s) => s.bbo);
  const privStatus = useConnectionStore(
    (s) => s.privateStatus,
  );
  const pubStatus = useConnectionStore(
    (s) => s.publicStatus,
  );
  const latency = useConnectionStore((s) => s.latency);

  const meta = symbols.get(selectedSymbol);
  const tickSize = meta?.tickSize ?? 0.01;
  const lotSize = meta?.lotSize ?? 0.001;

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

  return (
    <div
      className="h-12 bg-bg-surface border-b border-border
        flex items-center px-4 gap-4 shrink-0"
    >
      {/* Symbol selector */}
      <div className="relative" ref={dropRef}>
        <button
          className="text-text-primary font-semibold
            text-sm hover:text-accent transition-colors"
          onClick={() => setDropdownOpen(!dropdownOpen)}
          aria-expanded={dropdownOpen}
          aria-haspopup="listbox"
          aria-label="Select trading pair"
        >
          {meta?.name ?? "BTCUSDT"} &#9662;
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

      {/* Price stats */}
      <div className="flex items-center gap-3 text-xs">
        <span className="font-mono text-sm text-buy">
          {lastPx}
        </span>
        <div className="flex flex-col">
          <span className="text-text-secondary">Mark</span>
          <span className="font-mono text-text-primary">
            {bbo.askPx > 0
              ? formatPrice(
                  Math.round((bbo.bidPx + bbo.askPx) / 2),
                  tickSize,
                )
              : "--"}
          </span>
        </div>
        <div className="flex flex-col">
          <span className="text-text-secondary">Bid</span>
          <span className="font-mono text-buy">
            {bbo.bidPx > 0
              ? formatPrice(bbo.bidPx, tickSize)
              : "--"}
          </span>
        </div>
        <div className="flex flex-col">
          <span className="text-text-secondary">Ask</span>
          <span className="font-mono text-sell">
            {bbo.askPx > 0
              ? formatPrice(bbo.askPx, tickSize)
              : "--"}
          </span>
        </div>
        <div className="flex flex-col">
          <span className="text-text-secondary">
            Bid Size
          </span>
          <span className="font-mono text-text-primary">
            {bbo.bidQty > 0
              ? formatQty(bbo.bidQty, lotSize)
              : "--"}
          </span>
        </div>
        <div className="flex flex-col">
          <span className="text-text-secondary">
            Ask Size
          </span>
          <span className="font-mono text-text-primary">
            {bbo.askQty > 0
              ? formatQty(bbo.askQty, lotSize)
              : "--"}
          </span>
        </div>
      </div>

      {/* Spacer */}
      <div className="flex-1" />

      {/* Connection status */}
      <div
        className="flex items-center gap-2 text-xs"
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
