import { memo } from "react";
import clsx from "clsx";
import { useTradingStore } from "../../store/trading";
import { useMarketStore } from "../../store/market";
import { Side } from "../../lib/protocol";
import { cancelOrder } from "../../lib/protocol";
import { formatPrice } from "../../lib/format";
import { formatQty } from "../../lib/format";
import type { UserOrder } from "../../lib/types";
import type { SymbolMeta } from "../../lib/types";

const OrderRow = memo(function OrderRow({
  o,
  meta,
  send,
}: {
  o: UserOrder;
  meta: SymbolMeta | undefined;
  send: (msg: string) => void;
}) {
  const tick = meta?.tickSize ?? 0.01;
  const lot = meta?.lotSize ?? 0.001;
  const isBuy = o.side === Side.BUY;

  return (
    <tr
      className="border-t border-border
        hover:bg-bg-hover"
    >
      <td className="px-4 py-2">
        {meta?.name ?? o.symbolId}
      </td>
      <td
        className={clsx(
          "px-2 py-2",
          isBuy ? "text-buy" : "text-sell",
        )}
      >
        {isBuy ? "Buy" : "Sell"}
      </td>
      <td className="px-2 py-2
        text-text-secondary"
      >
        {o.postOnly
          ? "Post"
          : o.reduceOnly
            ? "RO"
            : "Limit"}
      </td>
      <td className="px-2 py-2 text-right">
        {formatPrice(o.price, tick)}
      </td>
      <td className="px-2 py-2 text-right">
        {formatQty(o.qty, lot)}
      </td>
      <td className="px-2 py-2 text-right">
        {formatQty(o.filled, lot)}
      </td>
      <td className="px-2 py-2 text-right">
        <button
          className="text-text-secondary
            hover:text-sell text-xs"
          onClick={() =>
            send(cancelOrder(o.oid))
          }
        >
          Cancel
        </button>
      </td>
    </tr>
  );
});

interface Props {
  send: (msg: string) => void;
}

export function OpenOrders({ send }: Props) {
  const orders = useTradingStore((s) => s.orders);
  const symbols = useMarketStore((s) => s.symbols);

  function cancelAll() {
    if (!window.confirm("Cancel all open orders?")) {
      return;
    }
    for (const o of orders) {
      send(cancelOrder(o.oid));
    }
  }

  if (orders.length === 0) {
    return (
      <div className="flex items-center justify-center
        h-full text-text-secondary text-sm"
      >
        No open orders
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      <div className="flex justify-end px-4 py-1">
        <button
          className="text-xs text-sell
            hover:text-sell/80"
          onClick={cancelAll}
        >
          Cancel All
        </button>
      </div>
      <table className="w-full text-xs font-mono">
        <thead>
          <tr className="text-text-secondary text-left">
            <th className="px-4 py-2">Symbol</th>
            <th className="px-2 py-2">Side</th>
            <th className="px-2 py-2">Type</th>
            <th className="px-2 py-2 text-right">
              Price
            </th>
            <th className="px-2 py-2 text-right">Qty</th>
            <th className="px-2 py-2 text-right">
              Filled
            </th>
            <th className="px-2 py-2"></th>
          </tr>
        </thead>
        <tbody>
          {orders.map((o) => (
            <OrderRow
              key={o.oid}
              o={o}
              meta={symbols.get(o.symbolId)}
              send={send}
            />
          ))}
        </tbody>
      </table>
    </div>
  );
}
