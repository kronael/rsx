import { useEffect, useCallback } from "react";
import { Side } from "../lib/protocol";
import { formatPrice } from "../lib/format";

interface KeyboardOptions {
  priceInputRef: React.RefObject<HTMLInputElement | null>;
  qtyInputRef: React.RefObject<HTMLInputElement | null>;
  onSetSide: (side: Side) => void;
  onSubmitBuy: () => void;
  onSubmitSell: () => void;
  priceStr: string;
  onSetPrice: (v: string) => void;
  tickSize: number;
  // When true, global B/S shortcuts are suppressed
  // (e.g. user is typing in a text field elsewhere)
  disabled?: boolean;
}

// Returns true when the event target is an input/textarea/select —
// we should not hijack B/S in those cases.
function isInputTarget(e: KeyboardEvent): boolean {
  const tag = (e.target as HTMLElement).tagName;
  return (
    tag === "INPUT" ||
    tag === "TEXTAREA" ||
    tag === "SELECT" ||
    (e.target as HTMLElement).isContentEditable
  );
}

export function useKeyboard({
  priceInputRef,
  qtyInputRef,
  onSetSide,
  onSubmitBuy,
  onSubmitSell,
  priceStr,
  onSetPrice,
  tickSize,
  disabled = false,
}: KeyboardOptions): void {
  // Adjust price by +/- one tick when price input is active.
  const adjustPrice = useCallback(
    (dir: 1 | -1) => {
      const val = parseFloat(priceStr);
      if (!Number.isFinite(val) || tickSize <= 0) return;
      const decimals = countDecimals(tickSize);
      const next = val + dir * tickSize;
      if (next <= 0) return;
      onSetPrice(next.toFixed(decimals));
    },
    [priceStr, tickSize, onSetPrice],
  );

  const handleGlobal = useCallback(
    (e: KeyboardEvent) => {
      if (disabled) return;
      // B / S: only fire when NOT already in an input
      if (!isInputTarget(e)) {
        if (e.key === "b" || e.key === "B") {
          e.preventDefault();
          onSetSide(Side.BUY);
          priceInputRef.current?.focus();
          return;
        }
        if (e.key === "s" || e.key === "S") {
          e.preventDefault();
          onSetSide(Side.SELL);
          priceInputRef.current?.focus();
          return;
        }
      }

      // Up / Down: only when price input is focused
      const priceActive =
        document.activeElement === priceInputRef.current;
      if (priceActive) {
        if (e.key === "ArrowUp") {
          e.preventDefault();
          adjustPrice(1);
          return;
        }
        if (e.key === "ArrowDown") {
          e.preventDefault();
          adjustPrice(-1);
          return;
        }
      }

      // Enter: submit when price or qty input is focused
      if (e.key === "Enter") {
        const priceOrQtyFocused =
          document.activeElement === priceInputRef.current ||
          document.activeElement === qtyInputRef.current;
        if (!priceOrQtyFocused) return;
        e.preventDefault();
        // Check which side is active via aria-pressed on buy button,
        // or fall back to submitting buy (caller decides side state)
        const activeEl = document.activeElement as HTMLElement;
        // Walk up to find the OrderEntry container and check
        // the active side via data attribute
        const form = activeEl.closest(
          "[data-order-side]",
        ) as HTMLElement | null;
        const side = form?.dataset["orderSide"];
        if (side === "sell") {
          onSubmitSell();
        } else {
          onSubmitBuy();
        }
      }
    },
    [
      disabled,
      priceInputRef,
      qtyInputRef,
      onSetSide,
      onSubmitBuy,
      onSubmitSell,
      adjustPrice,
    ],
  );

  useEffect(() => {
    window.addEventListener("keydown", handleGlobal);
    return () => {
      window.removeEventListener("keydown", handleGlobal);
    };
  }, [handleGlobal]);
}

function countDecimals(val: number): number {
  const str = val.toString();
  if (str.includes("e-")) {
    return parseInt(str.split("e-")[1] ?? "0", 10);
  }
  const dot = str.indexOf(".");
  if (dot < 0) return 0;
  return str.length - dot - 1;
}
