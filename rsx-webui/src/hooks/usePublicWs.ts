import { useEffect } from "react";
import { useRef } from "react";
import { parseMessage } from "../lib/protocol";
import { subscribe } from "../lib/protocol";
import { unsubscribe } from "../lib/protocol";
import { MdChannel } from "../lib/protocol";
import { WsStatus } from "../lib/types";
import { useConnectionStore } from "../store/connection";
import { useMarketStore } from "../store/market";

const ALL_CHANNELS =
  MdChannel.BBO | MdChannel.DEPTH | MdChannel.TRADES;

export function usePublicWs() {
  const wsRef = useRef<WebSocket | null>(null);
  const retryRef = useRef(1000);
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);
  const prevSymRef = useRef<number>(0);
  const selectedSymbol = useMarketStore(
    (s) => s.selectedSymbol,
  );

  useEffect(() => {
    let mounted = true;

    function connect() {
      if (!mounted) return;
      const setStatus =
        useConnectionStore.getState().setPublicStatus;
      setStatus(WsStatus.CONNECTING);

      const proto = location.protocol === "https:"
        ? "wss:" : "ws:";
      const url = `${proto}//${location.host}/ws/public`;
      const ws = new WebSocket(url);
      wsRef.current = ws;

      ws.onopen = () => {
        if (!mounted) return;
        setStatus(WsStatus.CONNECTED);
        retryRef.current = 1000;
        const sym = useMarketStore.getState().selectedSymbol;
        ws.send(subscribe(sym, ALL_CHANNELS));
        prevSymRef.current = sym;
      };

      ws.onmessage = (ev) => {
        const msg = parseMessage(ev.data as string);
        if (!msg) return;
        const store = useMarketStore.getState();

        if ("BBO" in msg) {
          const [, bidPx, bidQty, , askPx, askQty, , ts, seq]
            = msg.BBO;
          store.updateBbo(
            bidPx, bidQty, askPx, askQty, ts, seq,
          );
        } else if ("B" in msg) {
          const [, bids, asks, , seq] = msg.B;
          store.applyL2Snapshot(bids, asks, seq);
        } else if ("D" in msg) {
          const [, side, px, qty, count, , seq] = msg.D;
          store.applyL2Delta(side, px, qty, count, seq);
        } else if ("T" in msg) {
          const [, price, qty, side, ts, seq] = msg.T;
          store.addTrade({ price, qty, side, ts, seq });
        }
      };

      ws.onclose = () => {
        if (!mounted) return;
        setStatus(WsStatus.DISCONNECTED);
        const delay = retryRef.current;
        retryRef.current = Math.min(delay * 2, 30000);
        timerRef.current = setTimeout(connect, delay);
      };

      ws.onerror = () => {
        setStatus(WsStatus.ERROR);
      };
    }

    connect();

    return () => {
      mounted = false;
      if (timerRef.current) clearTimeout(timerRef.current);
      wsRef.current?.close();
    };
  }, []);

  // Re-subscribe on symbol change
  useEffect(() => {
    const ws = wsRef.current;
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    const prev = prevSymRef.current;
    if (prev === selectedSymbol) return;
    if (prev > 0) {
      ws.send(unsubscribe(prev, ALL_CHANNELS));
    }
    ws.send(subscribe(selectedSymbol, ALL_CHANNELS));
    prevSymRef.current = selectedSymbol;
  }, [selectedSymbol]);
}
