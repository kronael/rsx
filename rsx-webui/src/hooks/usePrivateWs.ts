import { useEffect } from "react";
import { useRef } from "react";
import { useCallback } from "react";
import { parseMessage } from "../lib/protocol";
import { heartbeat } from "../lib/protocol";
import { WsStatus } from "../lib/types";
import { useConnectionStore } from "../store/connection";
import { useTradingStore } from "../store/trading";

export function usePrivateWs() {
  const wsRef = useRef<WebSocket | null>(null);
  const retryRef = useRef(1000);
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);
  const hbRef = useRef<ReturnType<typeof setInterval>>(undefined);

  const send = useCallback((msg: string) => {
    const ws = wsRef.current;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(msg);
    }
  }, []);

  useEffect(() => {
    let mounted = true;

    function connect() {
      if (!mounted) return;
      const setStatus =
        useConnectionStore.getState().setPrivateStatus;
      setStatus(WsStatus.CONNECTING);

      const proto = location.protocol === "https:"
        ? "wss:" : "ws:";
      const url = `${proto}//${location.host}/ws/private`;
      const ws = new WebSocket(url);
      wsRef.current = ws;

      ws.onopen = () => {
        if (!mounted) return;
        setStatus(WsStatus.CONNECTED);
        retryRef.current = 1000;
        hbRef.current = setInterval(() => {
          send(heartbeat());
        }, 5000);
      };

      ws.onmessage = (ev) => {
        const msg = parseMessage(ev.data as string);
        if (!msg) return;
        if ("U" in msg) {
          const [oid, status, filled, remaining] = msg.U;
          useTradingStore.getState().updateOrder(
            oid, status, filled, remaining,
          );
        } else if ("F" in msg) {
          const [takerOid, makerOid, price, qty, ts, fee] =
            msg.F;
          useTradingStore.getState().addFill({
            takerOid, makerOid, price, qty, ts, fee,
          });
        } else if ("E" in msg) {
          console.error(
            `ws private error: ${msg.E[0]} ${msg.E[1]}`,
          );
        } else if ("H" in msg) {
          const rtt = Date.now() - msg.H[0];
          useConnectionStore.getState().setLatency(rtt);
        }
      };

      ws.onclose = () => {
        if (!mounted) return;
        cleanup();
        setStatus(WsStatus.DISCONNECTED);
        const delay = retryRef.current;
        retryRef.current = Math.min(delay * 2, 30000);
        timerRef.current = setTimeout(connect, delay);
      };

      ws.onerror = () => {
        setStatus(WsStatus.ERROR);
      };
    }

    function cleanup() {
      if (hbRef.current) clearInterval(hbRef.current);
    }

    connect();

    return () => {
      mounted = false;
      cleanup();
      if (timerRef.current) clearTimeout(timerRef.current);
      wsRef.current?.close();
    };
  }, [send]);

  return { send };
}
