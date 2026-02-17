import { useEffect } from "react";
import { useRef } from "react";
import { useTradingStore } from "../store/trading";
import { useSettingsStore } from "../store/settings";
import { playFillSound } from "../lib/sounds";
import { playLiquidationSound } from "../lib/sounds";

// Liquidation warning fires when markPx is within 10% of liqPx,
// at most once per 5 seconds per symbolId.
const LIQ_DISTANCE_THRESHOLD = 0.10;
const LIQ_DEBOUNCE_MS = 5000;

export function useSoundAlerts(): void {
  const fills = useTradingStore((s) => s.fills);
  const positions = useTradingStore((s) => s.positions);
  const soundEnabled = useSettingsStore((s) => s.soundEnabled);

  // Track previous fill to detect new arrivals and infer side from price.
  const prevFillRef = useRef<{ takerOid: string; price: number } | null>(null);

  // Track last liquidation warning time per symbolId.
  const liqLastFiredRef = useRef<Map<number, number>>(new Map());

  // Fill alert: fires when fills[0] changes (new fill prepended by addFill).
  useEffect(() => {
    if (!soundEnabled) return;
    if (fills.length === 0) return;

    const latest = fills[0];
    if (!latest) return;
    const prev = prevFillRef.current;

    const isNew = !prev || prev.takerOid !== latest.takerOid;
    if (!isNew) return;

    // Infer side from price direction relative to previous fill.
    // No side in UserFill, so use price comparison as best proxy.
    const side: "buy" | "sell" =
      prev && latest.price < prev.price ? "sell" : "buy";

    prevFillRef.current = {
      takerOid: latest.takerOid,
      price: latest.price,
    };

    playFillSound(side);
  }, [fills, soundEnabled]);

  // Liquidation warning: fires when position markPx is within 10% of liqPx.
  useEffect(() => {
    if (!soundEnabled) return;
    if (positions.length === 0) return;

    const now = Date.now();

    for (const pos of positions) {
      if (pos.liqPx <= 0) continue;
      if (pos.qty <= 0) continue;

      // Distance as fraction: |markPx - liqPx| / liqPx
      const distance =
        Math.abs(pos.markPx - pos.liqPx) / pos.liqPx;

      if (distance >= LIQ_DISTANCE_THRESHOLD) continue;

      const lastFired =
        liqLastFiredRef.current.get(pos.symbolId) ?? 0;

      if (now - lastFired < LIQ_DEBOUNCE_MS) continue;

      liqLastFiredRef.current.set(pos.symbolId, now);
      playLiquidationSound();
    }
  }, [positions, soundEnabled]);
}
