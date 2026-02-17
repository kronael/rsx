import { useEffect } from "react";
import { useRef } from "react";
import { useState } from "react";
import { useCallback } from "react";
import {
  createChart,
  type IChartApi,
  type ISeriesApi,
  type CandlestickData,
  type LineData,
  type Time,
  ColorType,
  LineStyle,
  CrosshairMode,
} from "lightweight-charts";
import clsx from "clsx";
import { useMarketStore } from "../../store/market";

const TIMEFRAMES: Record<string, number> = {
  "1m": 60,
  "5m": 300,
  "15m": 900,
  "1h": 3600,
  "4h": 14400,
  "1D": 86400,
};

interface Candle {
  open: number;
  high: number;
  low: number;
  close: number;
  volume: number;
  time: number;
}

// ── Indicator math ────────────────────────────────────────

function calcEma(closes: number[], period: number): number[] {
  if (closes.length === 0) return [];
  const k = 2 / (period + 1);
  const out: number[] = [];
  let ema = closes[0]!;
  for (let i = 0; i < closes.length; i++) {
    if (i < period - 1) {
      out.push(NaN);
      continue;
    }
    if (i === period - 1) {
      // seed with SMA
      let sum = 0;
      for (let j = 0; j < period; j++) sum += closes[j]!;
      ema = sum / period;
    } else {
      ema = closes[i]! * k + ema * (1 - k);
    }
    out.push(ema);
  }
  return out;
}

function calcBB(
  closes: number[],
  period: number,
  mult: number,
): { upper: number[]; mid: number[]; lower: number[] } {
  const upper: number[] = [];
  const mid: number[] = [];
  const lower: number[] = [];
  for (let i = 0; i < closes.length; i++) {
    if (i < period - 1) {
      upper.push(NaN);
      mid.push(NaN);
      lower.push(NaN);
      continue;
    }
    let sum = 0;
    for (let j = i - period + 1; j <= i; j++) {
      sum += closes[j]!;
    }
    const sma = sum / period;
    let variance = 0;
    for (let j = i - period + 1; j <= i; j++) {
      const d = closes[j]! - sma;
      variance += d * d;
    }
    const sd = Math.sqrt(variance / period);
    upper.push(sma + mult * sd);
    mid.push(sma);
    lower.push(sma - mult * sd);
  }
  return { upper, mid, lower };
}

function calcRsi(closes: number[], period: number): number[] {
  if (closes.length < period + 1) return closes.map(() => NaN);
  const out: number[] = new Array(closes.length).fill(NaN);
  let avgGain = 0;
  let avgLoss = 0;
  for (let i = 1; i <= period; i++) {
    const diff = closes[i]! - closes[i - 1]!;
    if (diff > 0) avgGain += diff;
    else avgLoss += Math.abs(diff);
  }
  avgGain /= period;
  avgLoss /= period;
  out[period] = avgLoss === 0
    ? 100
    : 100 - 100 / (1 + avgGain / avgLoss);
  for (let i = period + 1; i < closes.length; i++) {
    const diff = closes[i]! - closes[i - 1]!;
    const gain = diff > 0 ? diff : 0;
    const loss = diff < 0 ? Math.abs(diff) : 0;
    avgGain = (avgGain * (period - 1) + gain) / period;
    avgLoss = (avgLoss * (period - 1) + loss) / period;
    out[i] = avgLoss === 0
      ? 100
      : 100 - 100 / (1 + avgGain / avgLoss);
  }
  return out;
}

// ── Drawing tool types ────────────────────────────────────

type DrawTool = "none" | "hline" | "trendline";

interface HLine {
  price: number;
  id: string;
}

interface TrendLine {
  t1: number; // time (unix sec)
  p1: number; // price
  t2: number;
  p2: number;
  id: string;
}

// ── Chart component ───────────────────────────────────────

type IndicatorKey = "ema9" | "ema21" | "bb" | "rsi";

const IND_DEFAULTS: Record<IndicatorKey, boolean> = {
  ema9: false,
  ema21: false,
  bb: false,
  rsi: false,
};

export function Chart() {
  const containerRef = useRef<HTMLDivElement>(null);
  const rsiContainerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const rsiChartRef = useRef<IChartApi | null>(null);
  const candleRef =
    useRef<ISeriesApi<"Candlestick"> | null>(null);
  const volumeRef =
    useRef<ISeriesApi<"Histogram"> | null>(null);
  const ema9Ref =
    useRef<ISeriesApi<"Line"> | null>(null);
  const ema21Ref =
    useRef<ISeriesApi<"Line"> | null>(null);
  const bbUpperRef =
    useRef<ISeriesApi<"Line"> | null>(null);
  const bbMidRef =
    useRef<ISeriesApi<"Line"> | null>(null);
  const bbLowerRef =
    useRef<ISeriesApi<"Line"> | null>(null);
  const rsiSeriesRef =
    useRef<ISeriesApi<"Line"> | null>(null);
  // Trend line series keyed by id
  const trendSeriesRef =
    useRef<Map<string, ISeriesApi<"Line">>>(new Map());

  const candlesRef = useRef<Map<number, Candle>>(new Map());
  const [tf, setTf] = useState("1m");
  const tfRef = useRef(60);
  const meta = useMarketStore((s) => {
    const sel = s.selectedSymbol;
    return s.symbols.get(sel);
  });
  const tickSize = meta?.tickSize ?? 0.01;
  const tickRef = useRef(tickSize);
  tickRef.current = tickSize;

  const [indicators, setIndicators] =
    useState<Record<IndicatorKey, boolean>>(IND_DEFAULTS);
  const indicatorsRef =
    useRef<Record<IndicatorKey, boolean>>(IND_DEFAULTS);
  indicatorsRef.current = indicators;

  const [drawTool, setDrawTool] =
    useState<DrawTool>("none");
  const drawToolRef = useRef<DrawTool>("none");
  drawToolRef.current = drawTool;

  // Drawing state
  const hLinesRef = useRef<HLine[]>([]);
  const trendLinesRef = useRef<TrendLine[]>([]);
  const pendingTrendRef = useRef<{
    t1: number; p1: number;
  } | null>(null);

  // ── Create main chart ──────────────────────────────────
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const chart = createChart(el, {
      layout: {
        background: {
          type: ColorType.Solid,
          color: "#0b0e11",
        },
        textColor: "#848e9c",
        fontFamily: "JetBrains Mono, monospace",
      },
      grid: {
        vertLines: { color: "#1e2329" },
        horzLines: { color: "#1e2329" },
      },
      crosshair: {
        mode: CrosshairMode.Normal,
        vertLine: { color: "#363c45" },
        horzLine: { color: "#363c45" },
      },
      timeScale: {
        borderColor: "#2b3139",
        timeVisible: true,
      },
      rightPriceScale: {
        borderColor: "#2b3139",
      },
    });
    chartRef.current = chart;

    const candleSeries = chart.addCandlestickSeries({
      upColor: "#0ecb81",
      downColor: "#f6465d",
      borderUpColor: "#0ecb81",
      borderDownColor: "#f6465d",
      wickUpColor: "#0ecb81",
      wickDownColor: "#f6465d",
    });
    candleRef.current = candleSeries;

    const volumeSeries = chart.addHistogramSeries({
      color: "#2b3139",
      priceFormat: { type: "volume" },
      priceScaleId: "",
    });
    volumeSeries.priceScale().applyOptions({
      scaleMargins: { top: 0.8, bottom: 0 },
    });
    volumeRef.current = volumeSeries;

    // EMA 9
    const ema9 = chart.addLineSeries({
      color: "#f0b90b",
      lineWidth: 1,
      priceLineVisible: false,
      lastValueVisible: false,
      crosshairMarkerVisible: false,
    });
    ema9Ref.current = ema9;
    ema9.applyOptions({
      visible: indicatorsRef.current.ema9,
    });

    // EMA 21
    const ema21 = chart.addLineSeries({
      color: "#2196f3",
      lineWidth: 1,
      priceLineVisible: false,
      lastValueVisible: false,
      crosshairMarkerVisible: false,
    });
    ema21Ref.current = ema21;
    ema21.applyOptions({
      visible: indicatorsRef.current.ema21,
    });

    // BB upper
    const bbUpper = chart.addLineSeries({
      color: "rgba(128,128,255,0.6)",
      lineWidth: 1,
      lineStyle: LineStyle.Dashed,
      priceLineVisible: false,
      lastValueVisible: false,
      crosshairMarkerVisible: false,
    });
    bbUpperRef.current = bbUpper;
    bbUpper.applyOptions({
      visible: indicatorsRef.current.bb,
    });

    // BB mid
    const bbMid = chart.addLineSeries({
      color: "rgba(128,128,255,0.4)",
      lineWidth: 1,
      priceLineVisible: false,
      lastValueVisible: false,
      crosshairMarkerVisible: false,
    });
    bbMidRef.current = bbMid;
    bbMid.applyOptions({
      visible: indicatorsRef.current.bb,
    });

    // BB lower
    const bbLower = chart.addLineSeries({
      color: "rgba(128,128,255,0.6)",
      lineWidth: 1,
      lineStyle: LineStyle.Dashed,
      priceLineVisible: false,
      lastValueVisible: false,
      crosshairMarkerVisible: false,
    });
    bbLowerRef.current = bbLower;
    bbLower.applyOptions({
      visible: indicatorsRef.current.bb,
    });

    // Click handler for drawing tools
    chart.subscribeClick((param) => {
      const tool = drawToolRef.current;
      if (tool === "none" || !param.time || !param.point) {
        return;
      }
      const t = param.time as number;
      const p = candleSeries.coordinateToPrice(
        param.point.y,
      );
      if (p === null) return;

      if (tool === "hline") {
        const id = `hline-${Date.now()}`;
        hLinesRef.current.push({ price: p, id });
        candleSeries.createPriceLine({
          price: p,
          color: "#f0b90b",
          lineWidth: 1,
          lineStyle: LineStyle.Dashed,
          axisLabelVisible: true,
          title: "",
        });
      } else if (tool === "trendline") {
        const pending = pendingTrendRef.current;
        if (!pending) {
          pendingTrendRef.current = { t1: t, p1: p };
        } else {
          const id = `trend-${Date.now()}`;
          const tl: TrendLine = {
            t1: pending.t1,
            p1: pending.p1,
            t2: t,
            p2: p,
            id,
          };
          trendLinesRef.current.push(tl);
          pendingTrendRef.current = null;

          const series = chart.addLineSeries({
            color: "#f0b90b",
            lineWidth: 1,
            priceLineVisible: false,
            lastValueVisible: false,
            crosshairMarkerVisible: false,
          });
          series.setData([
            { time: tl.t1 as Time, value: tl.p1 },
            { time: tl.t2 as Time, value: tl.p2 },
          ]);
          trendSeriesRef.current.set(id, series);
        }
      }
    });

    const ro = new ResizeObserver(() => {
      chart.applyOptions({
        width: el.clientWidth,
        height: el.clientHeight,
      });
    });
    ro.observe(el);

    return () => {
      ro.disconnect();
      chart.remove();
      chartRef.current = null;
      candleRef.current = null;
      volumeRef.current = null;
      ema9Ref.current = null;
      ema21Ref.current = null;
      bbUpperRef.current = null;
      bbMidRef.current = null;
      bbLowerRef.current = null;
      trendSeriesRef.current.clear();
    };
  }, []);

  // ── Create RSI sub-chart ───────────────────────────────
  useEffect(() => {
    const el = rsiContainerRef.current;
    if (!el) return;

    const chart = createChart(el, {
      layout: {
        background: {
          type: ColorType.Solid,
          color: "#0b0e11",
        },
        textColor: "#848e9c",
        fontFamily: "JetBrains Mono, monospace",
      },
      grid: {
        vertLines: { color: "#1e2329" },
        horzLines: { color: "#1e2329" },
      },
      crosshair: {
        vertLine: { color: "#363c45" },
        horzLine: { color: "#363c45" },
      },
      timeScale: {
        borderColor: "#2b3139",
        timeVisible: true,
        visible: false,
      },
      rightPriceScale: {
        borderColor: "#2b3139",
        scaleMargins: { top: 0.1, bottom: 0.1 },
      },
      height: 100,
    });
    rsiChartRef.current = chart;

    const rsiSeries = chart.addLineSeries({
      color: "#9c27b0",
      lineWidth: 1,
      priceLineVisible: false,
      lastValueVisible: true,
    });
    // Overbought / oversold reference lines
    rsiSeries.createPriceLine({
      price: 70,
      color: "rgba(246,70,93,0.5)",
      lineWidth: 1,
      lineStyle: LineStyle.Dashed,
      axisLabelVisible: false,
      title: "70",
    });
    rsiSeries.createPriceLine({
      price: 30,
      color: "rgba(14,203,129,0.5)",
      lineWidth: 1,
      lineStyle: LineStyle.Dashed,
      axisLabelVisible: false,
      title: "30",
    });
    rsiSeriesRef.current = rsiSeries;

    const ro = new ResizeObserver(() => {
      chart.applyOptions({ width: el.clientWidth });
    });
    ro.observe(el);

    return () => {
      ro.disconnect();
      chart.remove();
      rsiChartRef.current = null;
      rsiSeriesRef.current = null;
    };
  }, []);

  // ── Recompute and push indicator data ─────────────────
  const recomputeIndicators = useCallback(() => {
    const candles = Array.from(
      candlesRef.current.values(),
    ).sort((a, b) => a.time - b.time);

    if (candles.length === 0) return;

    const closes = candles.map((c) => c.close);
    const times = candles.map((c) => c.time as Time);

    // EMA 9
    if (ema9Ref.current) {
      const vals = calcEma(closes, 9);
      const data: LineData<Time>[] = [];
      for (let i = 0; i < candles.length; i++) {
        const v = vals[i];
        if (v !== undefined && !isNaN(v)) {
          data.push({ time: times[i]!, value: v });
        }
      }
      ema9Ref.current.setData(data);
    }

    // EMA 21
    if (ema21Ref.current) {
      const vals = calcEma(closes, 21);
      const data: LineData<Time>[] = [];
      for (let i = 0; i < candles.length; i++) {
        const v = vals[i];
        if (v !== undefined && !isNaN(v)) {
          data.push({ time: times[i]!, value: v });
        }
      }
      ema21Ref.current.setData(data);
    }

    // BB
    if (bbUpperRef.current && bbMidRef.current &&
        bbLowerRef.current) {
      const { upper, mid, lower } =
        calcBB(closes, 20, 2);
      const uData: LineData<Time>[] = [];
      const mData: LineData<Time>[] = [];
      const lData: LineData<Time>[] = [];
      for (let i = 0; i < candles.length; i++) {
        const u = upper[i];
        const m = mid[i];
        const l = lower[i];
        if (
          u !== undefined && !isNaN(u) &&
          m !== undefined && !isNaN(m) &&
          l !== undefined && !isNaN(l)
        ) {
          uData.push({ time: times[i]!, value: u });
          mData.push({ time: times[i]!, value: m });
          lData.push({ time: times[i]!, value: l });
        }
      }
      bbUpperRef.current.setData(uData);
      bbMidRef.current.setData(mData);
      bbLowerRef.current.setData(lData);
    }

    // RSI
    if (rsiSeriesRef.current) {
      const vals = calcRsi(closes, 14);
      const data: LineData<Time>[] = [];
      for (let i = 0; i < candles.length; i++) {
        const v = vals[i];
        if (v !== undefined && !isNaN(v)) {
          data.push({ time: times[i]!, value: v });
        }
      }
      rsiSeriesRef.current.setData(data);
    }
  }, []);

  // ── Update candles from trade stream ──────────────────
  function updateCandle(
    price: number,
    qty: number,
    ts: number,
  ) {
    const humanPx = price * tickRef.current;
    const interval = tfRef.current;
    const tsSec = Math.floor(ts / 1_000_000_000);
    const bucket =
      Math.floor(tsSec / interval) * interval;
    const candles = candlesRef.current;

    let c = candles.get(bucket);
    if (!c) {
      c = {
        open: humanPx,
        high: humanPx,
        low: humanPx,
        close: humanPx,
        volume: qty,
        time: bucket,
      };
      candles.set(bucket, c);
      if (candles.size > 500) {
        const oldest = candles.keys().next().value;
        if (oldest !== undefined) candles.delete(oldest);
      }
    } else {
      c.high = Math.max(c.high, humanPx);
      c.low = Math.min(c.low, humanPx);
      c.close = humanPx;
      c.volume += qty;
    }

    const data: CandlestickData<Time> = {
      time: bucket as Time,
      open: c.open,
      high: c.high,
      low: c.low,
      close: c.close,
    };
    candleRef.current?.update(data);
    volumeRef.current?.update({
      time: bucket as Time,
      value: c.volume,
      color:
        c.close >= c.open
          ? "rgba(14,203,129,0.3)"
          : "rgba(246,70,93,0.3)",
    });

    // Incrementally update indicator last point
    const ind = indicatorsRef.current;
    if (!ind.ema9 && !ind.ema21 && !ind.bb && !ind.rsi) {
      return;
    }
    recomputeIndicators();
  }

  // Subscribe to trades via ring buffer.
  useEffect(() => {
    let prevTs = 0;
    const unsub = useMarketStore.subscribe((state) => {
      const ring = state.tradeRing;
      const newest = ring.newest();
      if (!newest || newest.ts === prevTs) return;
      prevTs = newest.ts;
      updateCandle(newest.price, newest.qty, newest.ts);
    });
    return unsub;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Reset candles on timeframe change
  useEffect(() => {
    tfRef.current = TIMEFRAMES[tf] ?? 60;
    candlesRef.current.clear();
    candleRef.current?.setData([]);
    volumeRef.current?.setData([]);
    ema9Ref.current?.setData([]);
    ema21Ref.current?.setData([]);
    bbUpperRef.current?.setData([]);
    bbMidRef.current?.setData([]);
    bbLowerRef.current?.setData([]);
    rsiSeriesRef.current?.setData([]);
    trendSeriesRef.current.forEach((s) => {
      chartRef.current?.removeSeries(s);
    });
    trendSeriesRef.current.clear();
    trendLinesRef.current = [];
    hLinesRef.current = [];
    pendingTrendRef.current = null;
  }, [tf]);

  // Toggle indicator visibility
  const toggleIndicator = useCallback(
    (key: IndicatorKey) => {
      setIndicators((prev) => {
        const next = { ...prev, [key]: !prev[key] };
        const on = next[key];
        switch (key) {
          case "ema9":
            ema9Ref.current?.applyOptions({
              visible: on,
            });
            break;
          case "ema21":
            ema21Ref.current?.applyOptions({
              visible: on,
            });
            break;
          case "bb":
            bbUpperRef.current?.applyOptions({
              visible: on,
            });
            bbMidRef.current?.applyOptions({
              visible: on,
            });
            bbLowerRef.current?.applyOptions({
              visible: on,
            });
            break;
          case "rsi":
            break;
        }
        if (on) recomputeIndicators();
        return next;
      });
    },
    [recomputeIndicators],
  );

  // Clear all drawn lines
  const clearDrawings = useCallback(() => {
    trendSeriesRef.current.forEach((s) => {
      chartRef.current?.removeSeries(s);
    });
    trendSeriesRef.current.clear();
    trendLinesRef.current = [];
    hLinesRef.current = [];
    pendingTrendRef.current = null;
    // Recreate candle series to clear price lines
    // (lightweight-charts has no removePriceLine API in v4)
  }, []);

  const rsiVisible = indicators.rsi;

  return (
    <div className="flex flex-col h-full">
      {/* Toolbar */}
      <div className="flex items-center gap-1 px-2 py-1
        bg-bg-surface border-b border-border shrink-0
        flex-wrap"
      >
        {/* Timeframe buttons */}
        {Object.keys(TIMEFRAMES).map((key) => (
          <button
            key={key}
            className={clsx(
              "px-2 py-0.5 text-xs rounded",
              key === tf
                ? "bg-bg-hover text-text-primary"
                : "text-text-secondary hover:text-text-primary",
            )}
            onClick={() => setTf(key)}
          >
            {key}
          </button>
        ))}

        <div className="w-px h-4 bg-border mx-1" />

        {/* Indicators */}
        {(
          [
            ["ema9", "EMA9"],
            ["ema21", "EMA21"],
            ["bb", "BB"],
            ["rsi", "RSI"],
          ] as [IndicatorKey, string][]
        ).map(([key, label]) => (
          <button
            key={key}
            className={clsx(
              "px-2 py-0.5 text-xs rounded",
              indicators[key]
                ? "bg-accent/20 text-accent"
                : "text-text-secondary hover:text-text-primary",
            )}
            onClick={() => toggleIndicator(key)}
            aria-pressed={indicators[key]}
          >
            {label}
          </button>
        ))}

        <div className="w-px h-4 bg-border mx-1" />

        {/* Drawing tools */}
        <button
          className={clsx(
            "px-2 py-0.5 text-xs rounded",
            drawTool === "hline"
              ? "bg-accent/20 text-accent"
              : "text-text-secondary hover:text-text-primary",
          )}
          onClick={() =>
            setDrawTool((d) =>
              d === "hline" ? "none" : "hline",
            )
          }
          title="Horizontal line: click chart to place"
          aria-pressed={drawTool === "hline"}
        >
          H-Line
        </button>
        <button
          className={clsx(
            "px-2 py-0.5 text-xs rounded",
            drawTool === "trendline"
              ? "bg-accent/20 text-accent"
              : "text-text-secondary hover:text-text-primary",
          )}
          onClick={() =>
            setDrawTool((d) =>
              d === "trendline" ? "none" : "trendline",
            )
          }
          title="Trend line: click two points on chart"
          aria-pressed={drawTool === "trendline"}
        >
          Trend
        </button>
        <button
          className="px-2 py-0.5 text-xs rounded
            text-text-secondary hover:text-sell"
          onClick={clearDrawings}
          title="Clear all drawings"
        >
          Clear
        </button>

        {/* Status indicator when drawing */}
        {drawTool !== "none" && (
          <span className="text-2xs text-accent ml-1">
            {drawTool === "hline"
              ? "Click to place line"
              : pendingTrendRef.current
                ? "Click 2nd point"
                : "Click 1st point"}
          </span>
        )}
      </div>

      {/* Main chart */}
      <div
        ref={containerRef}
        className={clsx(
          "flex-1",
          drawTool !== "none" && "cursor-crosshair",
        )}
      />

      {/* RSI sub-chart */}
      {rsiVisible && (
        <div
          ref={rsiContainerRef}
          className="h-[100px] border-t border-border shrink-0"
        />
      )}
    </div>
  );
}
