import { useState } from "react";
import { useEffect } from "react";
import { useCallback } from "react";
import { useRef } from "react";
import { useMarketStore } from "../../store/market";
import { formatPrice } from "../../lib/format";
import { formatTs } from "../../lib/format";
import { fetchFunding } from "../../hooks/useRestApi";

interface FundingEntry {
  ts: number;
  symbolId: number;
  amount: number;
  rate: number;
}

export function Funding() {
  const symbols = useMarketStore((s) => s.symbols);
  const selectedSymbol = useMarketStore(
    (s) => s.selectedSymbol,
  );
  const meta = symbols.get(selectedSymbol);
  const tickSize = meta?.tickSize ?? 0.01;

  const [entries, setEntries] = useState<FundingEntry[]>(
    [],
  );
  const [loading, setLoading] = useState(false);
  const [countdown, setCountdown] = useState("");

  // Load funding history
  useEffect(() => {
    setEntries([]);
    fetchFunding(selectedSymbol, 50)
      .then(setEntries)
      .catch(() => {});
  }, [selectedSymbol]);

  // Countdown to next funding (every 8h from midnight)
  useEffect(() => {
    function tick() {
      const now = Date.now();
      const h8 = 8 * 3600 * 1000;
      const next = Math.ceil(now / h8) * h8;
      const diff = next - now;
      const hours = Math.floor(diff / 3600000);
      const mins = Math.floor(
        (diff % 3600000) / 60000,
      );
      const secs = Math.floor((diff % 60000) / 1000);
      setCountdown(
        `${String(hours).padStart(2, "0")}:` +
        `${String(mins).padStart(2, "0")}:` +
        `${String(secs).padStart(2, "0")}`,
      );
    }
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, []);

  const entriesRef = useRef(entries);
  entriesRef.current = entries;

  const loadMore = useCallback(async () => {
    setLoading(true);
    try {
      const cur = entriesRef.current;
      const last = cur[cur.length - 1];
      const before = last ? String(last.ts) : undefined;
      const more = await fetchFunding(
        selectedSymbol, 50, before,
      );
      setEntries((prev) => [...prev, ...more]);
    } catch {
      // ignore
    } finally {
      setLoading(false);
    }
  }, [selectedSymbol]);

  const currentRate =
    entries.length > 0 ? entries[0] : null;

  return (
    <div className="flex flex-col h-full">
      {/* Current rate + countdown */}
      <div className="flex items-center gap-4 px-4 py-2
        bg-bg-surface border-b border-border text-xs"
      >
        <div>
          <span className="text-text-secondary">
            Funding Rate:{" "}
          </span>
          <span className="font-mono text-text-primary">
            {currentRate
              ? `${(currentRate.rate * 100).toFixed(4)}%`
              : "--"}
          </span>
        </div>
        <div>
          <span className="text-text-secondary">
            Next:{" "}
          </span>
          <span className="font-mono text-accent">
            {countdown}
          </span>
        </div>
      </div>

      {entries.length === 0 ? (
        <div className="flex items-center justify-center
          flex-1 text-text-secondary text-sm"
        >
          No funding history
        </div>
      ) : (
        <>
          <table className="w-full text-xs font-mono">
            <thead>
              <tr className="text-text-secondary
                text-left"
              >
                <th className="px-4 py-2">Time</th>
                <th className="px-2 py-2">Symbol</th>
                <th className="px-2 py-2 text-right">
                  Amount
                </th>
                <th className="px-2 py-2 text-right">
                  Rate
                </th>
              </tr>
            </thead>
            <tbody>
              {entries.map((e, i) => (
                <tr
                  key={`${e.ts}-${i}`}
                  className="border-t border-border
                    hover:bg-bg-hover"
                >
                  <td className="px-4 py-2
                    text-text-secondary"
                  >
                    {formatTs(e.ts)}
                  </td>
                  <td className="px-2 py-2">
                    {symbols.get(e.symbolId)?.name ??
                      e.symbolId}
                  </td>
                  <td className="px-2 py-2 text-right">
                    {formatPrice(e.amount, tickSize)}
                  </td>
                  <td className="px-2 py-2 text-right">
                    {(e.rate * 100).toFixed(4)}%
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="flex justify-center py-2">
            <button
              className="text-xs text-accent
                hover:text-accent/80
                disabled:opacity-50"
              disabled={loading}
              onClick={loadMore}
            >
              {loading ? "Loading..." : "Load More"}
            </button>
          </div>
        </>
      )}
    </div>
  );
}
