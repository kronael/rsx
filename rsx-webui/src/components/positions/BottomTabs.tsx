import { useState } from "react";
import clsx from "clsx";
import { useTradingStore } from "../../store/trading";
import { Positions } from "./Positions";
import { OpenOrders } from "./OpenOrders";
import { OrderHistory } from "./OrderHistory";
import { Funding } from "./Funding";
import { Assets } from "./Assets";

interface Props {
  send: (msg: string) => void;
}

type Tab =
  | "positions"
  | "orders"
  | "history"
  | "funding"
  | "assets";

export function BottomTabs({ send }: Props) {
  const [active, setActive] = useState<Tab>("positions");
  const posCount = useTradingStore(
    (s) => s.positions.length,
  );
  const orderCount = useTradingStore(
    (s) => s.orders.length,
  );

  const tabs: { key: Tab; label: string; count?: number }[]
    = [
      {
        key: "positions",
        label: "Positions",
        count: posCount,
      },
      {
        key: "orders",
        label: "Orders",
        count: orderCount,
      },
      { key: "history", label: "History" },
      { key: "funding", label: "Funding" },
      { key: "assets", label: "Assets" },
    ];

  return (
    <div className="flex flex-col h-full">
      <div className="flex gap-4 px-4 pt-2
        border-b border-border bg-bg-surface"
      >
        {tabs.map((t) => (
          <button
            key={t.key}
            className={clsx(
              "pb-2 text-sm flex items-center gap-1",
              active === t.key
                ? "tab-active"
                : "tab-inactive",
            )}
            onClick={() => setActive(t.key)}
          >
            {t.label}
            {t.count !== undefined && t.count > 0 && (
              <span className="text-2xs bg-bg-hover
                rounded px-1 py-0.5 font-mono"
              >
                {t.count}
              </span>
            )}
          </button>
        ))}
      </div>

      <div className="flex-1 overflow-auto">
        {active === "positions" && (
          <Positions send={send} />
        )}
        {active === "orders" && <OpenOrders send={send} />}
        {active === "history" && <OrderHistory />}
        {active === "funding" && <Funding />}
        {active === "assets" && <Assets />}
      </div>
    </div>
  );
}
