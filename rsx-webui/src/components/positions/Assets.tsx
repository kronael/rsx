import { useTradingStore } from "../../store/trading";
import { useMarketStore } from "../../store/market";
import { formatPnl } from "../../lib/format";
import { formatPrice } from "../../lib/format";
import { formatQty } from "../../lib/format";
import clsx from "clsx";

function Row({
  label,
  value,
  accent,
}: {
  label: string;
  value: string;
  accent?: "buy" | "sell" | "warn";
}) {
  return (
    <div className="flex items-center justify-between
      py-1.5 border-b border-border last:border-0"
    >
      <span className="text-text-secondary text-xs">
        {label}
      </span>
      <span
        className={clsx(
          "text-xs font-mono font-medium",
          accent === "buy" && "text-buy",
          accent === "sell" && "text-sell",
          accent === "warn" && "text-accent",
          !accent && "text-text-primary",
        )}
      >
        {value}
      </span>
    </div>
  );
}

export function Assets() {
  const account = useTradingStore((s) => s.account);
  const positions = useTradingStore((s) => s.positions);
  const symbols = useMarketStore((s) => s.symbols);

  // Aggregate unrealized PnL across all positions for display
  const totalUnrealizedPnl = positions.reduce(
    (sum, p) => {
      const meta = symbols.get(p.symbolId);
      const tick = meta?.tickSize ?? 0.01;
      return sum + p.unrealizedPnl * tick;
    },
    0,
  );

  // Use margin ratio to flag risk level
  const marginRatio =
    account.equity > 0
      ? account.mm / account.equity
      : 0;
  const marginAccent: "buy" | "sell" | "warn" =
    marginRatio >= 0.8
      ? "sell"
      : marginRatio >= 0.5
        ? "warn"
        : "buy";

  const pnl = totalUnrealizedPnl;
  const pnlText =
    pnl === 0
      ? "0.00"
      : `${pnl >= 0 ? "+" : ""}${pnl.toFixed(2)}`;
  const pnlAccent: "buy" | "sell" =
    pnl >= 0 ? "buy" : "sell";

  return (
    <div className="p-4 space-y-4">
      {/* Account summary */}
      <div>
        <h3 className="text-2xs text-text-secondary
          uppercase tracking-wide mb-2"
        >
          Account
        </h3>
        <div className="bg-bg-surface rounded px-3 py-1">
          <Row
            label="Equity"
            value={account.equity.toFixed(2)}
          />
          <Row
            label="Available Balance"
            value={account.available.toFixed(2)}
          />
          <Row
            label="Collateral"
            value={account.collateral.toFixed(2)}
          />
          <Row
            label="Unrealized PnL"
            value={pnlText}
            accent={positions.length > 0
              ? pnlAccent
              : undefined}
          />
        </div>
      </div>

      {/* Margin summary */}
      <div>
        <h3 className="text-2xs text-text-secondary
          uppercase tracking-wide mb-2"
        >
          Margin
        </h3>
        <div className="bg-bg-surface rounded px-3 py-1">
          <Row
            label="Initial Margin"
            value={account.im.toFixed(2)}
          />
          <Row
            label="Maintenance Margin"
            value={account.mm.toFixed(2)}
          />
          <Row
            label="Margin Ratio"
            value={
              account.equity > 0
                ? `${(marginRatio * 100).toFixed(2)}%`
                : "--"
            }
            accent={
              account.equity > 0 ? marginAccent : undefined
            }
          />
        </div>
      </div>

      {/* Per-position margin */}
      {positions.length > 0 && (
        <div>
          <h3 className="text-2xs text-text-secondary
            uppercase tracking-wide mb-2"
          >
            Positions
          </h3>
          <div className="bg-bg-surface rounded overflow-hidden">
            <table className="w-full text-xs font-mono">
              <thead>
                <tr className="text-text-secondary border-b
                  border-border"
                >
                  <th className="px-3 py-1.5 text-left
                    font-normal"
                  >
                    Symbol
                  </th>
                  <th className="px-3 py-1.5 text-right
                    font-normal"
                  >
                    Size
                  </th>
                  <th className="px-3 py-1.5 text-right
                    font-normal"
                  >
                    Liq Px
                  </th>
                  <th className="px-3 py-1.5 text-right
                    font-normal"
                  >
                    PnL
                  </th>
                </tr>
              </thead>
              <tbody>
                {positions.map((p) => {
                  const meta = symbols.get(p.symbolId);
                  const tick = meta?.tickSize ?? 0.01;
                  const lot = meta?.lotSize ?? 0.001;
                  const pnlFmt = formatPnl(
                    p.unrealizedPnl,
                    tick,
                  );
                  return (
                    <tr
                      key={p.symbolId}
                      className="border-t border-border
                        hover:bg-bg-hover"
                    >
                      <td className="px-3 py-1.5 text-left">
                        {meta?.name ?? String(p.symbolId)}
                      </td>
                      <td className="px-3 py-1.5 text-right">
                        {formatQty(p.qty, lot)}
                      </td>
                      <td className="px-3 py-1.5 text-right
                        text-text-secondary"
                      >
                        {p.liqPx > 0
                          ? formatPrice(p.liqPx, tick)
                          : "--"}
                      </td>
                      <td
                        className={clsx(
                          "px-3 py-1.5 text-right",
                          pnlFmt.positive
                            ? "text-buy"
                            : "text-sell",
                        )}
                      >
                        {pnlFmt.text}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </div>
      )}

      {positions.length === 0 && account.equity === 0 && (
        <p className="text-text-secondary text-xs
          text-center py-4"
        >
          No account data
        </p>
      )}
    </div>
  );
}
