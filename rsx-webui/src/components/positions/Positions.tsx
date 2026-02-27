import {
  memo,
  useState,
  useRef,
  useEffect,
  useCallback,
} from "react";
import clsx from "clsx";
import { useTradingStore } from "../../store/trading";
import { useMarketStore } from "../../store/market";
import { Side } from "../../lib/protocol";
import { TIF } from "../../lib/protocol";
import { newOrder } from "../../lib/protocol";
import { formatPrice } from "../../lib/format";
import { formatQty } from "../../lib/format";
import { formatPnl } from "../../lib/format";
import { generateCid } from "../../lib/format";
import type { UserPosition } from "../../lib/types";
import type { SymbolMeta } from "../../lib/types";

// ---------------------------------------------------------------------------
// ADL indicator: 1-5 lit bars based on proximity to liquidation.
// More bars = higher ADL risk (position closer to liq relative to entry).
// ---------------------------------------------------------------------------
function adlLevel(p: UserPosition): number {
  if (p.liqPx <= 0 || p.entryPx <= 0 || p.markPx <= 0) {
    return 1;
  }
  const isBuy = p.side === Side.BUY;
  // distance from mark to liq, as fraction of entry
  const liqDist = isBuy
    ? p.markPx - p.liqPx
    : p.liqPx - p.markPx;
  const entryDist = isBuy
    ? p.entryPx - p.liqPx
    : p.liqPx - p.entryPx;
  if (entryDist <= 0) return 5;
  const frac = 1 - Math.max(0, Math.min(1, liqDist / entryDist));
  // frac 0 = far from liq (level 1), frac 1 = at liq (level 5)
  return Math.max(1, Math.min(5, Math.ceil(frac * 5)));
}

function AdlBars({ level }: { level: number }) {
  return (
    <div
      className="flex items-center gap-[2px]"
      title={`ADL level ${level}/5`}
    >
      {[1, 2, 3, 4, 5].map((i) => (
        <div
          key={i}
          className={clsx(
            "w-[4px] rounded-[1px]",
            i <= level
              ? level >= 4
                ? "bg-sell h-[10px]"
                : level >= 3
                  ? "bg-accent h-[10px]"
                  : "bg-buy h-[10px]"
              : "bg-border h-[6px]",
          )}
        />
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Close modal
// ---------------------------------------------------------------------------
interface CloseModalProps {
  position: UserPosition;
  meta: SymbolMeta | undefined;
  onConfirm: () => void;
  onCancel: () => void;
}

function CloseModal({
  position,
  meta,
  onConfirm,
  onCancel,
}: CloseModalProps) {
  const tick = meta?.tickSize ?? 0.01;
  const lot = meta?.lotSize ?? 0.001;
  const isBuy = position.side === Side.BUY;

  // Close on Escape
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [onCancel]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center
        justify-center bg-black/60"
      role="dialog"
      aria-modal="true"
      aria-label="Close position"
    >
      <div
        className="bg-bg-surface border border-border
          rounded-lg p-5 w-[320px] shadow-xl"
      >
        <h3 className="text-sm font-semibold text-text-primary
          mb-3"
        >
          Close Position
        </h3>
        <div className="text-xs text-text-secondary space-y-1
          mb-4"
        >
          <div className="flex justify-between">
            <span>Symbol</span>
            <span className="text-text-primary font-mono">
              {meta?.name ?? position.symbolId}
            </span>
          </div>
          <div className="flex justify-between">
            <span>Side</span>
            <span
              className={clsx(
                "font-mono",
                isBuy ? "text-buy" : "text-sell",
              )}
            >
              {isBuy ? "Long" : "Short"}
            </span>
          </div>
          <div className="flex justify-between">
            <span>Size</span>
            <span className="font-mono text-text-primary">
              {formatQty(position.qty, lot)}
            </span>
          </div>
          <div className="flex justify-between">
            <span>Mark price</span>
            <span className="font-mono text-text-primary">
              {formatPrice(position.markPx, tick)}
            </span>
          </div>
        </div>
        <p className="text-xs text-text-secondary mb-4">
          Market {isBuy ? "sell" : "buy"} at best available
          price (IOC, reduce-only).
        </p>
        <div className="flex gap-2">
          <button
            className="flex-1 py-2 rounded text-sm
              bg-bg-hover text-text-secondary
              hover:text-text-primary"
            onClick={onCancel}
          >
            Cancel
          </button>
          <button
            className={clsx(
              "flex-1 py-2 rounded text-sm font-semibold",
              isBuy ? "btn-sell" : "btn-buy",
            )}
            onClick={onConfirm}
            autoFocus
          >
            Confirm Close
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Position row
// ---------------------------------------------------------------------------
const PositionRow = memo(function PositionRow({
  p,
  meta,
  onClose,
}: {
  p: UserPosition;
  meta: SymbolMeta | undefined;
  onClose: (p: UserPosition) => void;
}) {
  const tick = meta?.tickSize ?? 0.01;
  const lot = meta?.lotSize ?? 0.001;
  const pnl = formatPnl(p.unrealizedPnl, tick);
  const isBuy = p.side === Side.BUY;
  const adl = adlLevel(p);

  // PnL flash on change
  const pnlRef = useRef<HTMLTableCellElement>(null);
  const prevPnl = useRef(p.unrealizedPnl);
  useEffect(() => {
    const cell = pnlRef.current;
    if (
      cell &&
      prevPnl.current !== p.unrealizedPnl
    ) {
      const dir =
        p.unrealizedPnl > prevPnl.current
          ? "animate-flash-buy"
          : "animate-flash-sell";
      cell.classList.remove(
        "animate-flash-buy",
        "animate-flash-sell",
      );
      void cell.offsetWidth; // restart animation
      cell.classList.add(dir);
    }
    prevPnl.current = p.unrealizedPnl;
  }, [p.unrealizedPnl]);

  const handleClose = useCallback(() => {
    onClose(p);
  }, [onClose, p]);

  return (
    <tr className="border-t border-border hover:bg-bg-hover">
      <td className="px-4 py-2">
        {meta?.name ?? p.symbolId}
      </td>
      <td
        className={clsx(
          "px-2 py-2",
          isBuy ? "text-buy" : "text-sell",
        )}
      >
        {isBuy ? "Long" : "Short"}
      </td>
      <td className="px-2 py-2 text-right">
        {formatQty(p.qty, lot)}
      </td>
      <td className="px-2 py-2 text-right">
        {formatPrice(p.entryPx, tick)}
      </td>
      <td className="px-2 py-2 text-right">
        {formatPrice(p.markPx, tick)}
      </td>
      <td
        ref={pnlRef}
        className={clsx(
          "px-2 py-2 text-right font-mono",
          pnl.positive ? "text-buy" : "text-sell",
        )}
      >
        {pnl.text}
      </td>
      <td className="px-2 py-2 text-right">
        {p.liqPx > 0
          ? formatPrice(p.liqPx, tick)
          : "--"}
      </td>
      {/* ADL indicator */}
      <td className="px-2 py-2">
        <AdlBars level={adl} />
      </td>
      <td className="px-2 py-2 text-right">
        <button
          className="text-text-secondary hover:text-sell
            text-xs"
          onClick={handleClose}
        >
          Close
        </button>
      </td>
    </tr>
  );
});

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------
interface Props {
  send: (msg: string) => void;
}

export function Positions({ send }: Props) {
  const positions = useTradingStore((s) => s.positions);
  const positionsLoaded = useTradingStore(
    (s) => s.positionsLoaded,
  );
  const symbols = useMarketStore((s) => s.symbols);
  const [closing, setClosing] = useState<
    UserPosition | null
  >(null);

  const handleCloseRequest = useCallback(
    (p: UserPosition) => setClosing(p),
    [],
  );

  const handleConfirm = useCallback(() => {
    if (!closing) return;
    const mkt = useMarketStore.getState();
    const isBuy = closing.side === Side.BUY;
    const closeSide = isBuy ? Side.SELL : Side.BUY;
    const px = isBuy ? mkt.bbo.bidPx : mkt.bbo.askPx;
    if (px <= 0) {
      setClosing(null);
      return;
    }
    const msg = newOrder(
      closing.symbolId,
      closeSide,
      px,
      closing.qty,
      generateCid(),
      TIF.IOC,
      true,
      false,
    );
    send(msg);
    setClosing(null);
  }, [closing, send]);

  const handleCancel = useCallback(
    () => setClosing(null),
    [],
  );

  if (!positionsLoaded) {
    return (
      <div className="flex items-center justify-center
        h-full text-text-secondary text-sm"
      />
    );
  }

  if (positions.length === 0) {
    return (
      <div className="flex items-center justify-center
        h-full text-text-secondary text-sm"
      >
        No open positions
      </div>
    );
  }

  return (
    <>
      {closing && (
        <CloseModal
          position={closing}
          meta={symbols.get(closing.symbolId)}
          onConfirm={handleConfirm}
          onCancel={handleCancel}
        />
      )}
      <table className="w-full text-xs font-mono">
        <thead>
          <tr className="text-text-secondary text-left">
            <th className="px-4 py-2">Symbol</th>
            <th className="px-2 py-2">Side</th>
            <th className="px-2 py-2 text-right">Size</th>
            <th className="px-2 py-2 text-right">Entry</th>
            <th className="px-2 py-2 text-right">Mark</th>
            <th className="px-2 py-2 text-right">PnL</th>
            <th className="px-2 py-2 text-right">Liq</th>
            <th className="px-2 py-2">ADL</th>
            <th className="px-2 py-2"></th>
          </tr>
        </thead>
        <tbody>
          {positions.map((p) => (
            <PositionRow
              key={p.symbolId}
              p={p}
              meta={symbols.get(p.symbolId)}
              onClose={handleCloseRequest}
            />
          ))}
        </tbody>
      </table>
    </>
  );
}
