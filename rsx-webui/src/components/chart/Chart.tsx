import { useEffect } from "react";
import { useRef } from "react";
import { useState } from "react";
import {
  createChart,
  type IChartApi,
  type ISeriesApi,
  type CandlestickData,
  type Time,
  ColorType,
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

export function Chart() {
  const containerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const candleRef =
    useRef<ISeriesApi<"Candlestick"> | null>(null);
  const volumeRef =
    useRef<ISeriesApi<"Histogram"> | null>(null);
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

  // Create chart
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
    };
  }, []);

  // Update candles from trade stream
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
  }

  // Subscribe to trades
  useEffect(() => {
    let prevLen =
      useMarketStore.getState().trades.length;
    const unsub = useMarketStore.subscribe((state) => {
      const trades = state.trades;
      if (trades.length === 0) return;
      const newCount = trades.length - prevLen;
      prevLen = trades.length;
      if (newCount <= 0) return;
      for (let i = newCount - 1; i >= 0; i--) {
        const t = trades[i];
        if (t) {
          updateCandle(t.price, t.qty, t.ts);
        }
      }
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
  }, [tf]);

  return (
    <div className="flex flex-col h-full">
      {/* Timeframe buttons */}
      <div className="flex items-center gap-1 px-2 py-1
        bg-bg-surface border-b border-border"
      >
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
      </div>
      <div ref={containerRef} className="flex-1" />
    </div>
  );
}
