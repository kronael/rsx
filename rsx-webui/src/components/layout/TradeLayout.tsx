import { useState } from "react";
import { useCallback } from "react";
import { useRef } from "react";
import { usePrivateWs } from "../../hooks/usePrivateWs";
import { usePublicWs } from "../../hooks/usePublicWs";
import { useMarketStore } from "../../store/market";
import { formatPrice } from "../../lib/format";
import { Orderbook } from "../orderbook/Orderbook";
import { TradesTape } from "../trades/TradesTape";
import { Chart } from "../chart/Chart";
import { DepthChart } from "../chart/DepthChart";
import { OrderEntry } from "../order/OrderEntry";
import { BottomTabs } from "../positions/BottomTabs";

const BOTTOM_MIN = 120;
const BOTTOM_MAX = 600;
const BOTTOM_DEFAULT = 256;

export function TradeLayout() {
  const { send } = usePrivateWs();
  usePublicWs();

  const [showDepth, setShowDepth] = useState(false);
  const [clickedPrice, setClickedPrice] = useState<
    { value: string; ts: number } | undefined
  >(undefined);
  const [bottomH, setBottomH] = useState(BOTTOM_DEFAULT);
  const dragRef = useRef<{
    startY: number;
    startH: number;
  } | null>(null);

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

  const onDragStart = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      dragRef.current = { startY: e.clientY, startH: bottomH };

      const onMove = (ev: MouseEvent) => {
        if (!dragRef.current) return;
        // dragging up = larger bottom panel
        const delta = dragRef.current.startY - ev.clientY;
        const next = Math.min(
          BOTTOM_MAX,
          Math.max(BOTTOM_MIN, dragRef.current.startH + delta),
        );
        setBottomH(next);
      };
      const onUp = () => {
        dragRef.current = null;
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
      };
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    },
    [bottomH],
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
        <div className="min-h-[300px] md:min-h-0 flex flex-col">
          <div className="flex items-center gap-1 px-2 pt-1 shrink-0">
            <button
              className={`text-xs px-2 py-0.5 rounded transition-colors ${!showDepth ? "bg-accent text-bg-base" : "text-text-secondary hover:text-text-primary"}`}
              onClick={() => setShowDepth(false)}
            >
              Candles
            </button>
            <button
              className={`text-xs px-2 py-0.5 rounded transition-colors ${showDepth ? "bg-accent text-bg-base" : "text-text-secondary hover:text-text-primary"}`}
              onClick={() => setShowDepth(true)}
            >
              Depth
            </button>
          </div>
          <div className="flex-1 min-h-0">
            {showDepth ? <DepthChart /> : <Chart />}
          </div>
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

      {/* Bottom: Tabs (resizable) */}
      <div
        className="border-t border-border flex flex-col"
        style={{ height: bottomH }}
      >
        {/* Drag handle */}
        <div
          className="h-1.5 w-full cursor-row-resize
            shrink-0 flex items-center justify-center
            hover:bg-accent/20 group"
          onMouseDown={onDragStart}
          title="Drag to resize"
          role="separator"
          aria-orientation="horizontal"
          aria-label="Resize bottom panel"
        >
          <div
            className="w-8 h-0.5 rounded-full bg-border
              group-hover:bg-accent/60 transition-colors"
          />
        </div>
        <div className="flex-1 min-h-0">
          <BottomTabs send={send} />
        </div>
      </div>
    </div>
  );
}
