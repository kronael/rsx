import { useState } from "react";
import { useCallback } from "react";
import { usePrivateWs } from "../../hooks/usePrivateWs";
import { usePublicWs } from "../../hooks/usePublicWs";
import { useMarketStore } from "../../store/market";
import { formatPrice } from "../../lib/format";
import { Orderbook } from "../orderbook/Orderbook";
import { TradesTape } from "../trades/TradesTape";
import { Chart } from "../chart/Chart";
import { OrderEntry } from "../order/OrderEntry";
import { BottomTabs } from "../positions/BottomTabs";

export function TradeLayout() {
  const { send } = usePrivateWs();
  usePublicWs();

  const [clickedPrice, setClickedPrice] = useState<
    { value: string; ts: number } | undefined
  >(undefined);

  const onPriceClick = useCallback(
    (rawPrice: number) => {
      const st = useMarketStore.getState();
      const sym = st.symbols.get(st.selectedSymbol);
      const tick = sym?.tickSize ?? 0.01;
      setClickedPrice({
        value: formatPrice(rawPrice, tick),
        ts: Date.now(),
      });
    },
    [],
  );

  return (
    <div className="flex-1 min-h-0 flex flex-col">
      {/* Main grid */}
      <div
        className="flex-1 min-h-0
          grid grid-cols-1 md:grid-cols-[288px_1fr_320px]
          grid-rows-1"
      >
        {/* Left: Orderbook + Trades (hidden on mobile) */}
        <div
          className="hidden md:flex flex-col
            border-r border-border min-h-0"
        >
          <div className="flex-1 min-h-0 overflow-hidden">
            <Orderbook onPriceClick={onPriceClick} />
          </div>
          <div
            className="h-[200px] border-t border-border
              overflow-hidden"
          >
            <TradesTape />
          </div>
        </div>

        {/* Center: Chart */}
        <div className="min-h-[300px] md:min-h-0">
          <Chart />
        </div>

        {/* Right: OrderEntry */}
        <div
          className="border-l border-border overflow-y-auto"
        >
          <OrderEntry
            send={send}
            externalPrice={clickedPrice}
          />
        </div>
      </div>

      {/* Bottom: Tabs */}
      <div className="h-[256px] border-t border-border">
        <BottomTabs send={send} />
      </div>
    </div>
  );
}
