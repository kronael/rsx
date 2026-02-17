import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import clsx from "clsx";
import { Side } from "../../lib/protocol";
import { TIF } from "../../lib/protocol";
import { newOrder } from "../../lib/protocol";
import { useMarketStore } from "../../store/market";
import { useBbo } from "../../store/market";
import { useSymbolMeta } from "../../store/market";
import { useTradingStore } from "../../store/trading";
import { parsePrice } from "../../lib/format";
import { parseQty } from "../../lib/format";
import { generateCid } from "../../lib/format";
import { useKeyboard } from "../../hooks/useKeyboard";

interface Props {
  send: (msg: string) => void;
  externalPrice?: { value: string; ts: number };
}

export function OrderEntry({ send, externalPrice }: Props) {
  const [orderType, setOrderType] = useState<
    "limit" | "market"
  >("limit");
  const [priceStr, setPriceStr] = useState("");
  const [qtyStr, setQtyStr] = useState("");
  const [tif, setTif] = useState<TIF>(TIF.GTC);
  const [reduceOnly, setReduceOnly] = useState(false);
  const [postOnly, setPostOnly] = useState(false);
  const [error, setError] = useState("");
  const [activePct, setActivePct] = useState<number | null>(
    null,
  );
  const [leverage, setLeverage] = useState(10);
  const [tpStr, setTpStr] = useState("");
  const [slStr, setSlStr] = useState("");
  const [side, setSide] = useState<Side>(Side.BUY);

  const priceInputRef = useRef<HTMLInputElement>(null);
  const qtyInputRef = useRef<HTMLInputElement>(null);

  const selectedSymbol = useMarketStore(
    (s) => s.selectedSymbol,
  );
  const bbo = useBbo();
  const meta = useSymbolMeta();
  const available = useTradingStore(
    (s) => s.account.available,
  );

  const tickSize = meta?.tickSize ?? 0.01;
  const lotSize = meta?.lotSize ?? 0.001;

  // Available balance in human USDT
  const availableHuman =
    available > 0
      ? (available * tickSize).toFixed(2)
      : "0.00";

  // Effective raw price for cost preview
  const effectivePxRaw = useMemo(() => {
    const mid = Math.round((bbo.bidPx + bbo.askPx) / 2);
    if (orderType === "market") return mid;
    const parsed = parsePrice(priceStr, tickSize);
    return parsed > 0 ? parsed : mid;
  }, [
    orderType, bbo.bidPx, bbo.askPx, priceStr, tickSize,
  ]);

  // Order cost: notional / leverage in USDT
  const orderCost = useMemo(() => {
    const qty = parseQty(qtyStr, lotSize);
    if (qty <= 0 || effectivePxRaw <= 0 || leverage <= 0) {
      return null;
    }
    const notional =
      effectivePxRaw * tickSize * qty * lotSize;
    return notional / leverage;
  }, [qtyStr, lotSize, effectivePxRaw, tickSize, leverage]);

  // Sync price from external source (orderbook click)
  useEffect(() => {
    if (externalPrice !== undefined) {
      setPriceStr(externalPrice.value);
    }
  }, [externalPrice]);

  const sliderPcts = [25, 50, 75, 100];

  const handleSubmit = useCallback((side: Side) => {
    setError("");
    const qty = parseQty(qtyStr, lotSize);
    if (qty <= 0) {
      setError("Enter a valid quantity");
      return;
    }

    let px: number;
    if (orderType === "market") {
      px = side === Side.BUY ? bbo.askPx : bbo.bidPx;
      if (px <= 0) {
        setError("No market price available");
        return;
      }
      const age = Date.now() - bbo.ts / 1_000_000;
      if (age > 5000) {
        setError("Market data stale");
        return;
      }
    } else {
      px = parsePrice(priceStr, tickSize);
      if (px <= 0) {
        setError("Enter a valid price");
        return;
      }
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
    setActivePct(null);
  }, [
    orderType, priceStr, qtyStr,
    tif, reduceOnly, postOnly,
    selectedSymbol, bbo, tickSize, lotSize, send,
  ]);

  const isLimit = orderType === "limit";

  const applyPct = useCallback((pct: number) => {
    if (available <= 0) return;
    // Use mid price for pct calculation when no price entered
    const px = orderType === "market"
      ? Math.round((bbo.bidPx + bbo.askPx) / 2)
      : parsePrice(priceStr, tickSize);
    if (px <= 0) return;
    // With leverage, max notional = available * tickSize * lev
    const maxNotional =
      available * tickSize * leverage;
    const humanPx = px * tickSize;
    if (humanPx <= 0) return;
    const humanQty = (maxNotional * (pct / 100)) / humanPx;
    const decimals =
      lotSize.toString().split(".")[1]?.length ?? 0;
    setQtyStr(humanQty.toFixed(decimals));
    setActivePct(pct);
  }, [
    available, orderType, bbo, priceStr,
    tickSize, lotSize, leverage,
  ]);

  useKeyboard({
    priceInputRef,
    qtyInputRef,
    onSetSide: setSide,
    onSubmitBuy: () => handleSubmit(Side.BUY),
    onSubmitSell: () => handleSubmit(Side.SELL),
    priceStr,
    onSetPrice: setPriceStr,
    tickSize,
  });

  return (
    <div
      className="flex flex-col gap-3 p-3"
      data-order-side={
        side === Side.BUY ? "buy" : "sell"
      }
    >
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

      {/* Available balance */}
      <div className="flex justify-between text-xs
        text-text-secondary"
      >
        <span>Available</span>
        <span className="font-mono text-text-primary">
          {availableHuman} USDT
        </span>
      </div>

      {/* Leverage selector */}
      <div className="flex items-center gap-2">
        <span className="text-xs text-text-secondary shrink-0">
          Leverage
        </span>
        <div className="flex gap-1 flex-wrap flex-1
          justify-end"
        >
          {[1, 2, 5, 10, 20, 50, 100].map((lv) => (
            <button
              key={lv}
              className={clsx(
                "text-2xs px-1.5 py-0.5 rounded",
                leverage === lv
                  ? "bg-accent text-bg-primary"
                    + " font-semibold"
                  : "bg-bg-hover text-text-secondary"
                    + " hover:text-text-primary",
              )}
              onClick={() => setLeverage(lv)}
            >
              {lv}x
            </button>
          ))}
        </div>
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
            ref={priceInputRef}
            type="text"
            className="input-field w-full font-mono"
            placeholder="Price"
            value={priceStr}
            onChange={(e) => {
              setPriceStr(e.target.value);
              setActivePct(null);
            }}
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
          ref={qtyInputRef}
          type="text"
          className="input-field w-full font-mono"
          placeholder="Qty"
          value={qtyStr}
          onChange={(e) => {
            setQtyStr(e.target.value);
            setActivePct(null);
          }}
        />
      </div>

      {/* % slider buttons */}
      <div className="flex gap-1">
        {sliderPcts.map((pct) => (
          <button
            key={pct}
            className={clsx(
              "flex-1 text-2xs py-1 rounded font-mono",
              "border transition-colors",
              activePct === pct
                ? "border-accent text-accent bg-accent/10"
                : "border-border bg-bg-hover text-text-secondary hover:text-text-primary hover:border-text-secondary",
            )}
            onClick={() => applyPct(pct)}
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

      {/* Order cost preview */}
      <div className="flex justify-between text-xs
        text-text-secondary"
      >
        <span>Order Cost</span>
        <span className="font-mono text-text-primary">
          {orderCost !== null
            ? `${orderCost.toFixed(2)} USDT`
            : "--"}
        </span>
      </div>

      {/* TP / SL (placeholder — not wired to protocol yet) */}
      <div className="grid grid-cols-2 gap-2">
        <div>
          <label className="text-xs text-text-secondary
            mb-1 block"
          >
            Take Profit
          </label>
          <input
            type="text"
            className="input-field w-full font-mono
              focus:border-buy"
            placeholder="TP Price"
            value={tpStr}
            onChange={(e) => setTpStr(e.target.value)}
            aria-label="Take profit price"
          />
        </div>
        <div>
          <label className="text-xs text-text-secondary
            mb-1 block"
          >
            Stop Loss
          </label>
          <input
            type="text"
            className="input-field w-full font-mono
              focus:border-sell"
            placeholder="SL Price"
            value={slStr}
            onChange={(e) => setSlStr(e.target.value)}
            aria-label="Stop loss price"
          />
        </div>
      </div>

      {error && (
        <p className="text-xs text-sell">{error}</p>
      )}

      {/* Stacked buy / sell submit buttons */}
      <div className="flex flex-col gap-1">
        <button
          className="w-full py-2.5 rounded font-semibold
            text-sm btn-buy"
          onClick={() => handleSubmit(Side.BUY)}
        >
          Buy {isLimit ? "Limit" : "Market"}
        </button>
        <button
          className="w-full py-2.5 rounded font-semibold
            text-sm btn-sell"
          onClick={() => handleSubmit(Side.SELL)}
        >
          Sell {isLimit ? "Limit" : "Market"}
        </button>
      </div>
    </div>
  );
}
