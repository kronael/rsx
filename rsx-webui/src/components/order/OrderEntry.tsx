import { useState } from "react";
import { useEffect } from "react";
import { useCallback } from "react";
import clsx from "clsx";
import { Side } from "../../lib/protocol";
import { TIF } from "../../lib/protocol";
import { newOrder } from "../../lib/protocol";
import { useMarketStore } from "../../store/market";
import { useTradingStore } from "../../store/trading";
import { formatPrice } from "../../lib/format";
import { parsePrice } from "../../lib/format";
import { parseQty } from "../../lib/format";
import { generateCid } from "../../lib/format";

interface Props {
  send: (msg: string) => void;
  externalPrice?: { value: string; ts: number };
}

export function OrderEntry({ send, externalPrice }: Props) {
  const [side, setSide] = useState<Side>(Side.BUY);
  const [orderType, setOrderType] = useState<
    "limit" | "market"
  >("limit");
  const [priceStr, setPriceStr] = useState("");
  const [qtyStr, setQtyStr] = useState("");
  const [tif, setTif] = useState<TIF>(TIF.GTC);
  const [reduceOnly, setReduceOnly] = useState(false);
  const [postOnly, setPostOnly] = useState(false);

  const symbols = useMarketStore((s) => s.symbols);
  const selectedSymbol = useMarketStore(
    (s) => s.selectedSymbol,
  );
  const bbo = useMarketStore((s) => s.bbo);
  const available = useTradingStore(
    (s) => s.account.available,
  );

  const meta = symbols.get(selectedSymbol);
  const tickSize = meta?.tickSize ?? 0.01;
  const lotSize = meta?.lotSize ?? 0.001;

  // Sync price from external source (orderbook click)
  useEffect(() => {
    if (externalPrice !== undefined) {
      setPriceStr(externalPrice.value);
    }
  }, [externalPrice]);

  const sliderPcts = [25, 50, 75, 100];

  const handleSubmit = useCallback(() => {
    const qty = parseQty(qtyStr, lotSize);
    if (qty <= 0) return;

    let px: number;
    if (orderType === "market") {
      px = side === Side.BUY ? bbo.askPx : bbo.bidPx;
      if (px <= 0) return;
    } else {
      px = parsePrice(priceStr, tickSize);
      if (px <= 0) return;
    }

    const cid = generateCid();
    const msg = newOrder(
      selectedSymbol,
      side,
      px,
      qty,
      cid,
      orderType === "market" ? TIF.IOC : tif,
      reduceOnly,
      orderType === "limit" ? postOnly : false,
    );
    send(msg);
    setQtyStr("");
  }, [
    side, orderType, priceStr, qtyStr,
    tif, reduceOnly, postOnly,
    selectedSymbol, bbo, tickSize, lotSize, send,
  ]);

  const isLimit = orderType === "limit";
  const isBuy = side === Side.BUY;

  return (
    <div className="flex flex-col gap-3 p-3">
      {/* Order type tabs */}
      <div className="flex gap-2 text-sm">
        <button
          className={clsx(
            "pb-1",
            isLimit ? "tab-active" : "tab-inactive",
          )}
          onClick={() => setOrderType("limit")}
        >
          Limit
        </button>
        <button
          className={clsx(
            "pb-1",
            !isLimit ? "tab-active" : "tab-inactive",
          )}
          onClick={() => setOrderType("market")}
        >
          Market
        </button>
      </div>

      {/* Side toggle */}
      <div className="grid grid-cols-2 gap-1">
        <button
          className={clsx(
            "py-1.5 rounded text-sm font-semibold",
            isBuy
              ? "bg-buy text-bg-primary"
              : "bg-bg-hover text-text-secondary",
          )}
          onClick={() => setSide(Side.BUY)}
        >
          Buy
        </button>
        <button
          className={clsx(
            "py-1.5 rounded text-sm font-semibold",
            !isBuy
              ? "bg-sell text-white"
              : "bg-bg-hover text-text-secondary",
          )}
          onClick={() => setSide(Side.SELL)}
        >
          Sell
        </button>
      </div>

      {/* Available balance */}
      <div className="flex justify-between text-xs
        text-text-secondary"
      >
        <span>Available</span>
        <span className="font-mono text-text-primary">
          {available > 0
            ? formatPrice(available, tickSize)
            : "0.00"}
        </span>
      </div>

      {/* Price input (limit only) */}
      {isLimit && (
        <div>
          <label className="text-xs text-text-secondary
            mb-1 block"
          >
            Price
          </label>
          <input
            type="text"
            className="input-field w-full font-mono"
            placeholder="Price"
            value={priceStr}
            onChange={(e) => setPriceStr(e.target.value)}
          />
        </div>
      )}

      {/* Qty input */}
      <div>
        <label className="text-xs text-text-secondary
          mb-1 block"
        >
          Quantity
        </label>
        <input
          type="text"
          className="input-field w-full font-mono"
          placeholder="Qty"
          value={qtyStr}
          onChange={(e) => setQtyStr(e.target.value)}
        />
      </div>

      {/* Qty slider */}
      <div className="flex gap-1">
        {sliderPcts.map((pct) => (
          <button
            key={pct}
            className="flex-1 text-2xs py-1 rounded
              bg-bg-hover text-text-secondary
              hover:text-text-primary"
            onClick={() => {
              if (available <= 0) return;
              const px = orderType === "market"
                ? (side === Side.BUY
                  ? bbo.askPx : bbo.bidPx)
                : parsePrice(priceStr, tickSize);
              if (px <= 0) return;
              const notional = available * (pct / 100);
              const humanPx = px * tickSize;
              if (humanPx <= 0) return;
              const humanQty = notional / humanPx;
              const decimals =
                lotSize.toString().split(".")[1]
                  ?.length ?? 0;
              setQtyStr(humanQty.toFixed(decimals));
            }}
          >
            {pct}%
          </button>
        ))}
      </div>

      {/* Limit-only options */}
      {isLimit && (
        <>
          {/* TIF */}
          <div className="flex items-center gap-2">
            <label className="text-xs text-text-secondary">
              TIF
            </label>
            <select
              className="input-field text-xs flex-1"
              value={tif}
              onChange={(e) =>
                setTif(Number(e.target.value) as TIF)
              }
            >
              <option value={TIF.GTC}>GTC</option>
              <option value={TIF.IOC}>IOC</option>
              <option value={TIF.FOK}>FOK</option>
            </select>
          </div>

          {/* Checkboxes */}
          <div className="flex gap-4 text-xs
            text-text-secondary"
          >
            <label className="flex items-center gap-1
              cursor-pointer"
            >
              <input
                type="checkbox"
                checked={reduceOnly}
                onChange={(e) =>
                  setReduceOnly(e.target.checked)
                }
                className="accent-accent"
              />
              Reduce-only
            </label>
            <label className="flex items-center gap-1
              cursor-pointer"
            >
              <input
                type="checkbox"
                checked={postOnly}
                onChange={(e) =>
                  setPostOnly(e.target.checked)
                }
                className="accent-accent"
              />
              Post-only
            </label>
          </div>
        </>
      )}

      {/* Reduce-only for market */}
      {!isLimit && (
        <label className="flex items-center gap-1
          text-xs text-text-secondary cursor-pointer"
        >
          <input
            type="checkbox"
            checked={reduceOnly}
            onChange={(e) =>
              setReduceOnly(e.target.checked)
            }
            className="accent-accent"
          />
          Reduce-only
        </label>
      )}

      {/* Submit */}
      <button
        className={clsx(
          "w-full py-2 rounded font-semibold text-sm",
          isBuy ? "btn-buy" : "btn-sell",
        )}
        onClick={handleSubmit}
      >
        {isBuy ? "Buy" : "Sell"}{" "}
        {orderType === "limit" ? "Limit" : "Market"}
      </button>
    </div>
  );
}
