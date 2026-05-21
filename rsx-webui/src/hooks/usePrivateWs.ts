import { useEffect } from "react";
import { useRef } from "react";
import { useCallback } from "react";
import { parseMessage } from "../lib/protocol";
import { heartbeat } from "../lib/protocol";
import { WsStatus } from "../lib/types";
import { useConnectionStore } from "../store/connection";
import { useTradingStore } from "../store/trading";
import { useToastStore } from "../lib/toast";
import { getToken } from "../lib/auth";
import { decodeClaims } from "../lib/auth";
import { isExpired } from "../lib/auth";
import { fetchPositions } from "./useRestApi";

export function usePrivateWs() {
  const wsRef = useRef<WebSocket | null>(null);
  const retryRef = useRef(1000);
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);
  const hbRef = useRef<ReturnType<typeof setInterval>>(undefined);
  // Tracks whether the most recent connect() saw a valid
  // token in storage. Distinguishes a real auth failure
  // (post sign-in) from "not yet signed in" (no toast).
  const hadTokenRef = useRef(false);

  const send = useCallback((msg: string) => {
    const ws = wsRef.current;
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(msg);
    }
  }, []);

  useEffect(() => {
    let mounted = true;

    function hasValidToken(): boolean {
      const token = getToken();
      if (!token) return false;
      const claims = decodeClaims(token);
      return claims !== null && !isExpired(claims);
    }

    function connect() {
      if (!mounted) return;
      const setStatus =
        useConnectionStore.getState().setPrivateStatus;
      // Record whether the user has a token before opening
      // the socket. The playground proxy mints a guest JWT
      // for loopback callers, so we still try to connect
      // without a token — but a close(4001) without a token
      // is "logged out", not "credentials wrong".
      hadTokenRef.current = hasValidToken();
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
          // Real auth failure: only surface as an error if
          // the user actually had a token when we opened the
          // socket. Otherwise this is just "not signed in"
          // and the AuthButton already communicates that.
          if (hadTokenRef.current) {
            setStatus(WsStatus.ERROR);
            useToastStore.getState().add(
              "Authentication failed — check credentials",
              "error",
            );
          } else {
            setStatus(WsStatus.DISCONNECTED);
          }
          // Long backoff: pinging the gateway every second
          // when the user is logged out is wasteful.
          timerRef.current = setTimeout(connect, 30000);
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
          // Only toast on the first disconnect after a
          // successful session; routine reconnect cycles
          // (which can happen on dev server restarts) should
          // not spam the user. The status pill is the
          // primary signal.
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
