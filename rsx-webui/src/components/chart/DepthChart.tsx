/**
 * DepthChart — SVG cumulative bid/ask depth curves.
 *
 * Renders up to DEPTH_LEVELS levels on each side.
 * Bids grow right-to-left from mid; asks grow left-to-right.
 * Hover shows price + cumulative volume tooltip.
 * Pure SVG, no extra deps.
 */
import {
  useRef,
  useState,
  useMemo,
  useCallback,
} from "react";
import { useOrderbook } from "../../store/market";
import { useSymbolMeta } from "../../store/market";
import type { PriceLevel } from "../../lib/types";

const DEPTH_LEVELS = 50;
const PADDING = { top: 12, right: 16, bottom: 28, left: 12 };
const BID_COLOR = "#26a65b";    // green
const ASK_COLOR = "#e74c3c";    // red
const BID_FILL = "rgba(38,166,91,0.12)";
const ASK_FILL = "rgba(231,76,60,0.12)";

interface Point {
  px: number;   // raw tick price
  cum: number;  // cumulative qty (raw lots)
}

interface HoverInfo {
  x: number;
  y: number;
  price: string;
  cumVol: string;
  side: "bid" | "ask";
}

// Build step-function points for one side.
// bids: high price first, cumulative grows left-to-right in price space
// asks: low price first, cumulative grows left-to-right in price space
function toPoints(
  levels: PriceLevel[],
  limit: number,
): Point[] {
  const pts: Point[] = [];
  for (let i = 0; i < Math.min(levels.length, limit); i++) {
    pts.push({
      px: levels[i]!.price,
      cum: levels[i]!.total,
    });
  }
  return pts;
}

// Map data coords to SVG pixel coords.
function makeScales(
  bidPts: Point[],
  askPts: Point[],
  width: number,
  height: number,
) {
  const innerW = width - PADDING.left - PADDING.right;
  const innerH = height - PADDING.top - PADDING.bottom;

  const allPx = [
    ...bidPts.map((p) => p.px),
    ...askPts.map((p) => p.px),
  ];
  const allCum = [
    ...bidPts.map((p) => p.cum),
    ...askPts.map((p) => p.cum),
  ];

  const minPx = allPx.length > 0 ? Math.min(...allPx) : 0;
  const maxPx = allPx.length > 0 ? Math.max(...allPx) : 1;
  const maxCum = allCum.length > 0 ? Math.max(...allCum) : 1;
  const pxRange = maxPx - minPx || 1;

  const toX = (px: number) =>
    PADDING.left + ((px - minPx) / pxRange) * innerW;
  const toY = (cum: number) =>
    PADDING.top + innerH - (cum / maxCum) * innerH;

  return { toX, toY, minPx, maxPx, maxCum, innerW, innerH };
}

// Build an SVG path for a step curve.
// For bids: points are ordered highest-price first.
// We walk left, stepping down.
// For asks: points are ordered lowest-price first.
// We walk right, stepping up.
function stepPath(
  pts: Point[],
  toX: (px: number) => number,
  toY: (cum: number) => number,
  side: "bid" | "ask",
  innerH: number,
): string {
  if (pts.length === 0) return "";
  const baseline = PADDING.top + innerH;

  const segments: string[] = [];

  if (side === "bid") {
    // Bids: sorted highest price first (index 0 = best bid).
    // Step curve: at each price level, horizontal to that price,
    // then vertical to that cumulative.
    const startX = toX(pts[0]!.px);
    segments.push(`M ${startX} ${baseline}`);
    segments.push(`V ${toY(pts[0]!.cum)}`);

    for (let i = 1; i < pts.length; i++) {
      const x = toX(pts[i]!.px);
      const y = toY(pts[i]!.cum);
      segments.push(`H ${x}`);
      segments.push(`V ${y}`);
    }
    // close down to baseline
    void toX(pts[pts.length - 1]!.px); // lastX unused
    segments.push(`V ${baseline}`);
    segments.push(`H ${startX}`);
    segments.push("Z");
  } else {
    // Asks: sorted lowest price first (index 0 = best ask).
    const startX = toX(pts[0]!.px);
    segments.push(`M ${startX} ${baseline}`);
    segments.push(`V ${toY(pts[0]!.cum)}`);

    for (let i = 1; i < pts.length; i++) {
      const x = toX(pts[i]!.px);
      const y = toY(pts[i]!.cum);
      segments.push(`H ${x}`);
      segments.push(`V ${y}`);
    }
    void toX(pts[pts.length - 1]!.px); // lastX unused
    segments.push(`V ${baseline}`);
    segments.push(`H ${startX}`);
    segments.push("Z");
  }

  return segments.join(" ");
}

interface Props {
  height?: number;
}

export function DepthChart({ height = 180 }: Props) {
  const containerRef = useRef<SVGSVGElement>(null);
  const [hover, setHover] = useState<HoverInfo | null>(null);
  const [width, setWidth] = useState(400);

  // Measure container width via ResizeObserver
  const wrapRef = useCallback((el: HTMLDivElement | null) => {
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width;
      if (w && w > 0) setWidth(w);
    });
    ro.observe(el);
    setWidth(el.clientWidth || 400);
  }, []);

  const orderbook = useOrderbook();
  const meta = useSymbolMeta();
  const tickSize = meta?.tickSize ?? 0.01;
  const lotSize = meta?.lotSize ?? 0.001;

  const { bidPts, askPts } = useMemo(() => {
    // bids: highest first (already sorted desc in store)
    const b = toPoints(orderbook.bids, DEPTH_LEVELS);
    // asks: lowest first (already sorted asc in store)
    const a = toPoints(orderbook.asks, DEPTH_LEVELS);
    return { bidPts: b, askPts: a };
  }, [orderbook.bids, orderbook.asks]);

  const scales = useMemo(
    () => makeScales(bidPts, askPts, width, height),
    [bidPts, askPts, width, height],
  );

  const { toX, toY, innerH } = scales;

  const bidPath = useMemo(
    () => stepPath(bidPts, toX, toY, "bid", innerH),
    [bidPts, toX, toY, innerH],
  );
  const askPath = useMemo(
    () => stepPath(askPts, toX, toY, "ask", innerH),
    [askPts, toX, toY, innerH],
  );

  // Price axis ticks (5 evenly spaced)
  const priceTicks = useMemo(() => {
    const { minPx, maxPx } = scales;
    const n = 5;
    const ticks: { x: number; label: string }[] = [];
    for (let i = 0; i <= n; i++) {
      const rawPx = minPx + (i / n) * (maxPx - minPx);
      const humanPx = rawPx * tickSize;
      const decimals = tickSize.toString().includes(".")
        ? tickSize.toString().split(".")[1]!.length
        : 0;
      ticks.push({
        x: toX(rawPx),
        label: humanPx.toFixed(decimals),
      });
    }
    return ticks;
  }, [scales, tickSize, toX]);

  // Hit-test on mouse move: find nearest data point
  const handleMouseMove = useCallback(
    (e: React.MouseEvent<SVGSVGElement>) => {
      const svg = containerRef.current;
      if (!svg) return;
      const rect = svg.getBoundingClientRect();
      const mx = e.clientX - rect.left;
      const my = e.clientY - rect.top;

      // Find closest point in bidPts and askPts by x distance
      let best: HoverInfo | null = null;
      let bestDist = Infinity;

      const check = (
        pts: Point[],
        side: "bid" | "ask",
      ) => {
        for (const p of pts) {
          const px = toX(p.px);
          const dist = Math.abs(mx - px);
          if (dist < bestDist) {
            bestDist = dist;
            const humanPx = (p.px * tickSize).toFixed(
              tickSize.toString().includes(".")
                ? tickSize.toString().split(".")[1]!.length
                : 0,
            );
            const humanCum = (p.cum * lotSize).toFixed(
              lotSize.toString().includes(".")
                ? lotSize.toString().split(".")[1]!.length
                : 3,
            );
            best = {
              x: px,
              y: my,
              price: humanPx,
              cumVol: humanCum,
              side,
            };
          }
        }
      };

      check(bidPts, "bid");
      check(askPts, "ask");

      setHover(best);
    },
    [bidPts, askPts, toX, tickSize, lotSize],
  );

  const handleMouseLeave = useCallback(() => {
    setHover(null);
  }, []);

  const isEmpty = bidPts.length === 0 && askPts.length === 0;

  return (
    <div
      ref={wrapRef}
      className="w-full relative select-none"
      style={{ height }}
    >
      {isEmpty ? (
        <div className="flex items-center justify-center
          h-full text-text-secondary text-xs"
        >
          No depth data
        </div>
      ) : (
        <svg
          ref={containerRef}
          width={width}
          height={height}
          onMouseMove={handleMouseMove}
          onMouseLeave={handleMouseLeave}
          aria-label="Depth chart"
          role="img"
        >
          {/* Bid fill area */}
          {bidPath && (
            <path d={bidPath} fill={BID_FILL} />
          )}
          {/* Ask fill area */}
          {askPath && (
            <path d={askPath} fill={ASK_FILL} />
          )}
          {/* Bid stroke */}
          {bidPath && (
            <path
              d={bidPath}
              fill="none"
              stroke={BID_COLOR}
              strokeWidth={1.5}
            />
          )}
          {/* Ask stroke */}
          {askPath && (
            <path
              d={askPath}
              fill="none"
              stroke={ASK_COLOR}
              strokeWidth={1.5}
            />
          )}

          {/* Price axis */}
          <line
            x1={PADDING.left}
            y1={PADDING.top + innerH}
            x2={width - PADDING.right}
            y2={PADDING.top + innerH}
            stroke="rgba(255,255,255,0.1)"
            strokeWidth={1}
          />
          {priceTicks.map((t) => (
            <g key={t.label}>
              <line
                x1={t.x}
                y1={PADDING.top + innerH}
                x2={t.x}
                y2={PADDING.top + innerH + 4}
                stroke="rgba(255,255,255,0.2)"
                strokeWidth={1}
              />
              <text
                x={t.x}
                y={PADDING.top + innerH + 16}
                textAnchor="middle"
                fontSize={9}
                fill="rgba(255,255,255,0.4)"
                fontFamily="monospace"
              >
                {t.label}
              </text>
            </g>
          ))}

          {/* Hover crosshair + dot */}
          {hover && (
            <>
              <line
                x1={hover.x}
                y1={PADDING.top}
                x2={hover.x}
                y2={PADDING.top + innerH}
                stroke="rgba(255,255,255,0.25)"
                strokeWidth={1}
                strokeDasharray="3 3"
              />
              <circle
                cx={hover.x}
                cy={toY(
                  (hover.side === "bid" ? bidPts : askPts)
                    .find(
                      (p) =>
                        (p.px * tickSize).toFixed(2) ===
                        hover.price,
                    )?.cum ?? 0,
                )}
                r={4}
                fill={
                  hover.side === "bid"
                    ? BID_COLOR
                    : ASK_COLOR
                }
                stroke="var(--color-bg-surface, #1a1a2e)"
                strokeWidth={1.5}
              />
            </>
          )}
        </svg>
      )}

      {/* Tooltip */}
      {hover && (
        <div
          className="absolute pointer-events-none
            bg-bg-surface border border-border rounded
            px-2 py-1 text-xs font-mono shadow-lg z-10"
          style={{
            left: Math.min(
              hover.x + 10,
              width - 130,
            ),
            top: 8,
          }}
        >
          <div
            className={
              hover.side === "bid"
                ? "text-buy"
                : "text-sell"
            }
          >
            {hover.side === "bid" ? "Bid" : "Ask"}
          </div>
          <div className="text-text-secondary">
            Price:{" "}
            <span className="text-text-primary">
              {hover.price}
            </span>
          </div>
          <div className="text-text-secondary">
            Cum vol:{" "}
            <span className="text-text-primary">
              {hover.cumVol}
            </span>
          </div>
        </div>
      )}
    </div>
  );
}
