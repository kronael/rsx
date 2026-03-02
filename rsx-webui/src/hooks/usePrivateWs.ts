import { useEffect } from "react";
import { useRef } from "react";
import { useCallback } from "react";
import { parseMessage } from "../lib/protocol";
import { heartbeat } from "../lib/protocol";
import { WsStatus } from "../lib/types";
import { useConnectionStore } from "../store/connection";
import { useTradingStore } from "../store/trading";
import { useToastStore } from "../lib/toast";
import { fetchPositions } from "./useRestApi";

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
      const base = location.pathname.replace(
        /\/trade\/.*$/, "",
      );
      const url = `${proto}//${location.host}${base}/ws/private`;
      const ws = new WebSocket(url);
      wsRef.current = ws;

      ws.onopen = () => {
        if (!mounted) return;
        retryRef.current = 1000;
        setStatus(WsStatus.CONNECTED);
        hbRef.current = setInterval(() => {
          if (
            wsRef.current &&
            wsRef.current.readyState === WebSocket.OPEN
          ) {
            wsRef.current.send(heartbeat());
          }
        }, 5000);
        fetchPositions().then((pos) => {
          if (!mounted) return;
          useTradingStore.getState().setPositions(pos);
        }).catch(() => {
          if (!mounted) return;
          useTradingStore.getState().setPositions([]);
        });
      };

      ws.onmessage = (ev) => {
        const msg = parseMessage(ev.data as string);
        if (!msg) return;
        if ("U" in msg) {
          const u = msg.U as unknown[];
          // Position updates: entries are objects with type:"position"
          if (
            u.length > 0 &&
            typeof u[0] === "object" &&
            u[0] !== null &&
            (u[0] as Record<string, unknown>).type ===
              "position"
          ) {
            const store = useTradingStore.getState();
            for (const entry of u) {
              const p = entry as {
                type: string;
                symbolId: number;
                side: number;
                qty: number;
                entryPx: number;
                markPx: number;
                unrealizedPnl: number;
                liqPx: number;
              };
              store.updatePosition({
                symbolId: p.symbolId,
                side: p.side,
                qty: p.qty,
                entryPx: p.entryPx,
                markPx: p.markPx,
                unrealizedPnl: p.unrealizedPnl,
                liqPx: p.liqPx,
              });
            }
          } else {
            const [oid, status, filled, remaining] =
              u as [string, number, number, number];
            useTradingStore.getState().updateOrder(
              oid, status, filled, remaining,
            );
          }
        } else if ("F" in msg) {
          const [takerOid, makerOid, price, qty, ts, fee] =
            msg.F;
          useTradingStore.getState().addFill({
            takerOid, makerOid, price, qty, ts, fee,
          });
          // Delay 150ms: WAL flush interval ~10ms,
          // allow recorder to persist before REST read.
          setTimeout(() => {
            if (!mounted) return;
            fetchPositions().then((pos) => {
              if (!mounted) return;
              useTradingStore.getState().setPositions(pos);
            }).catch(() => {/* ignore */});
          }, 150);
        } else if ("E" in msg) {
          console.error(
            `ws private error: ${msg.E[0]} ${msg.E[1]}`,
          );
          useToastStore.getState().add(
            msg.E[1] || `Error ${msg.E[0]}`, "error",
          );
        } else if ("H" in msg) {
          const rtt = Date.now() - msg.H[0];
          useConnectionStore.getState().setLatency(rtt);
        }
      };

      ws.onclose = (ev) => {
        if (!mounted) return;
        cleanup();
        if (ev.code === 4001) {
          setStatus(WsStatus.ERROR);
          useToastStore.getState().add(
            "Authentication failed — check credentials", "error",
          );
          return;
        }
        if (ev.code === 1013) {
          setStatus(WsStatus.OFFLINE);
          useToastStore.getState().add(
            "Exchange offline", "error",
          );
          retryRef.current = 30000;
        } else {
          setStatus(WsStatus.RECONNECTING);
          useToastStore.getState().add(
            "Private WS disconnected", "error",
          );
        }
        const delay = retryRef.current;
        retryRef.current = Math.min(delay * 2, 30000);
        timerRef.current = setTimeout(connect, delay);
      };

      ws.onerror = () => {
        setStatus(WsStatus.ERROR);
      };
    }

    function cleanup() {
      if (hbRef.current) {
        clearInterval(hbRef.current);
        hbRef.current = undefined;
      }
    }

    connect();

    return () => {
      mounted = false;
      cleanup();
      if (timerRef.current) clearTimeout(timerRef.current);
      wsRef.current?.close();
    };
  }, []);

  return { send };
}
