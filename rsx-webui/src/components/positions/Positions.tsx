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

interface Props {
  send: (msg: string) => void;
}

export function Positions({ send }: Props) {
  const positions = useTradingStore((s) => s.positions);
  const symbols = useMarketStore((s) => s.symbols);

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
          <th className="px-2 py-2"></th>
        </tr>
      </thead>
      <tbody>
        {positions.map((p) => {
          const meta = symbols.get(p.symbolId);
          const tick = meta?.tickSize ?? 0.01;
          const lot = meta?.lotSize ?? 0.001;
          const pnl = formatPnl(p.unrealizedPnl, tick);
          const isBuy = p.side === Side.BUY;

          return (
            <tr
              key={p.symbolId}
              className="border-t border-border
                hover:bg-bg-hover"
            >
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
                className={clsx(
                  "px-2 py-2 text-right",
                  pnl.positive
                    ? "text-buy"
                    : "text-sell",
                )}
              >
                {pnl.text}
              </td>
              <td className="px-2 py-2 text-right">
                {p.liqPx > 0
                  ? formatPrice(p.liqPx, tick)
                  : "--"}
              </td>
              <td className="px-2 py-2 text-right">
                <button
                  className="text-text-secondary
                    hover:text-sell text-xs"
                  onClick={() => {
                    const closeSide = isBuy
                      ? Side.SELL
                      : Side.BUY;
                    const mkt =
                      useMarketStore.getState();
                    if (
                      mkt.selectedSymbol !== p.symbolId
                    ) return;
                    const px = isBuy
                      ? mkt.bbo.bidPx
                      : mkt.bbo.askPx;
                    if (px <= 0) return;
                    const msg = newOrder(
                      p.symbolId,
                      closeSide,
                      px,
                      p.qty,
                      generateCid(),
                      TIF.IOC,
                      true,
                      false,
                    );
                    send(msg);
                  }}
                >
                  Close
                </button>
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
