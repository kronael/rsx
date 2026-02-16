import { useState } from "react";
import { useCallback } from "react";
import { useMemo } from "react";
import { useRef } from "react";
import { useTradingStore } from "../../store/trading";
import { useMarketStore } from "../../store/market";
import { formatPrice } from "../../lib/format";
import { formatQty } from "../../lib/format";
import { formatTs } from "../../lib/format";
import { fetchFills } from "../../hooks/useRestApi";
import type { UserFill } from "../../lib/types";

export function OrderHistory() {
  const fills = useTradingStore((s) => s.fills);
  const symbols = useMarketStore((s) => s.symbols);
  const selectedSymbol = useMarketStore(
    (s) => s.selectedSymbol,
  );
  const [loading, setLoading] = useState(false);
  const [older, setOlder] = useState<UserFill[]>([]);

  const meta = symbols.get(selectedSymbol);
  const tickSize = meta?.tickSize ?? 0.01;
  const lotSize = meta?.lotSize ?? 0.001;

  const allFills = useMemo(
    () => [...fills, ...older],
    [fills, older],
  );
  const allFillsRef = useRef(allFills);
  allFillsRef.current = allFills;

  const loadMore = useCallback(async () => {
    setLoading(true);
    try {
      const cur = allFillsRef.current;
      const last = cur[cur.length - 1];
      const before = last ? String(last.ts) : undefined;
      const more = await fetchFills(
        selectedSymbol, 50, before,
      );
      setOlder((prev) => [...prev, ...more]);
    } catch {
      // ignore
    } finally {
      setLoading(false);
    }
  }, [selectedSymbol]);

  if (allFills.length === 0) {
    return (
      <div className="flex items-center justify-center
        h-full text-text-secondary text-sm"
      >
        No fill history
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      <table className="w-full text-xs font-mono">
        <thead>
          <tr className="text-text-secondary text-left">
            <th className="px-4 py-2">Time</th>
            <th className="px-2 py-2 text-right">
              Price
            </th>
            <th className="px-2 py-2 text-right">Qty</th>
            <th className="px-2 py-2 text-right">Fee</th>
          </tr>
        </thead>
        <tbody>
          {allFills.map((f, i) => (
            <tr
              key={`${f.ts}-${i}`}
              className="border-t border-border
                hover:bg-bg-hover"
            >
              <td className="px-4 py-2 text-text-secondary">
                {formatTs(f.ts)}
              </td>
              <td className="px-2 py-2 text-right">
                {formatPrice(f.price, tickSize)}
              </td>
              <td className="px-2 py-2 text-right">
                {formatQty(f.qty, lotSize)}
              </td>
              <td className="px-2 py-2 text-right
                text-text-secondary"
              >
                {formatPrice(f.fee, tickSize)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="flex justify-center py-2">
        <button
          className="text-xs text-accent
            hover:text-accent/80 disabled:opacity-50"
          disabled={loading}
          onClick={loadMore}
        >
          {loading ? "Loading..." : "Load More"}
        </button>
      </div>
    </div>
  );
}
