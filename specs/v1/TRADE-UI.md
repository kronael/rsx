# TRADE-UI.md

Trade UI integration issues and fix plan.

## Current State

### What Works
- React SPA serves from rsx-webui/dist/ at /trade via playground
- Playground proxies /ws/private (gateway :8080) and /ws/public
  (marketdata :8180) with user-id injection
- Direct access to playground port 49171: WS connects, data flows
- Order entry, orderbook, fills visible on direct port

### What Is Broken
1. WS disconnected errors through nginx (market data + private)
2. Docs page 502 through nginx (works on direct port)
3. Open positions not displayed in trade UI
4. WS reconnect may silently fail after proxy drop

---

## Root Cause Analysis

### 1. WS disconnected through nginx

nginx default config does not forward the `Upgrade` and `Connection`
headers required for HTTP→WS upgrade. Requests arrive at playground
as plain HTTP; playground returns 400 or stalls; browser shows
"WS disconnected".

Required nginx directives (missing):
```
proxy_http_version 1.1;
proxy_set_header Upgrade $http_upgrade;
proxy_set_header Connection "upgrade";
```

Without these, nginx closes the connection before the 101 Switching
Protocols response reaches the browser.

### 2. Docs 502

Docs route proxied to a port or path not currently bound. Server.py
serves docs at a separate path but nginx location block points to
wrong upstream or dead port. Result: 502 Bad Gateway.

### 3. No positions display

The trade UI does not request current positions on connect. The
gateway private WS delivers position snapshots only on `{N:[...]}`:
subscribe with channel `positions`. The UI may subscribe to fills
and orders but omit positions channel, so no `{U:[...]}` position
updates arrive after login.

Alternatively: `{U:[...]}` delivers incremental updates only; the
initial snapshot requires an explicit `{N:[positions]}` subscription
or a REST `GET /v1/positions` call on connect.

### 4. WS reconnect logic

On disconnect (code 1013 from proxy = gateway not running, or 1006
abnormal close from nginx timeout), the UI WebSocket `onclose`
handler must schedule a retry. If the handler does not check `wasClean`
or retries without exponential backoff, it may reconnect immediately,
hit the same nginx misconfiguration, and give up after N attempts.

---

## Fix Plan

### Fix 1: nginx WS upgrade

Location blocks for /ws/private and /ws/public must include:

```nginx
location /ws/ {
    proxy_pass http://127.0.0.1:49171;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
    proxy_read_timeout 3600s;
    proxy_send_timeout 3600s;
}
```

`proxy_read_timeout` must exceed heartbeat interval (5s) by a large
margin; default 60s is usually fine but explicit is safer.

### Fix 2: Docs 502

Verify the upstream port/path for docs in nginx config. If docs are
served by playground on port 49171 at /docs, the nginx location must
point to `http://127.0.0.1:49171/docs`, not a separate process.
Check and align the nginx upstream address with where server.py
actually serves the docs route.

### Fix 3: Position display

On private WS connect, after auth, send:

```json
{"N": ["positions", "orders", "fills"]}
```

Gateway responds with snapshot for each channel. The UI must handle
`{U:[...]}` messages with `type: "position"` and render them in the
Positions component.

If snapshot is empty (no open positions), display "No open positions"
rather than a blank panel. This distinguishes "not subscribed" from
"subscribed, zero positions".

Fallback: `GET /v1/positions` on connect if WS snapshot is absent
after 2s timeout.

### Fix 4: WS reconnect

Reconnect strategy:
- On `onclose`: schedule retry with exponential backoff
  (1s, 2s, 4s, 8s, cap 30s)
- Reset backoff counter on successful message received
- Display connection state in UI: connecting / live / reconnecting
- On code 1013 (gateway not running): show "Exchange offline",
  continue retrying with 30s cap
- Do not retry indefinitely without user feedback

---

## Implementation Tasks

1. Update nginx config: add WS upgrade headers to /ws/ location block
2. Fix nginx docs upstream: align location with server.py docs path
3. rsx-webui: add `positions` to initial subscribe message on connect
4. rsx-webui: handle `{U:[...]}` position messages in Positions component
5. rsx-webui: show "No open positions" when snapshot returns empty
6. rsx-webui: implement exponential backoff reconnect in WS hook
7. rsx-webui: add connection status indicator (connecting/live/offline)

---

## Acceptance Criteria

- [ ] Orderbook and BBO stream live through nginx (no WS disconnect)
- [ ] Private WS connects through nginx, orders submit successfully
- [ ] Docs page loads through nginx (no 502)
- [ ] Positions panel shows open positions after login
- [ ] Positions panel shows "No open positions" when flat
- [ ] WS reconnects automatically after network drop with backoff
- [ ] UI shows connection state (live/reconnecting/offline)
- [ ] Direct port access continues to work (no regression)

---

## Testing

```
make smoke     # verifies WS connect + order round-trip
```

Manual checklist:
1. Load /trade through nginx, verify orderbook streams
2. Submit order, verify fill and order update received
3. Open positions tab after fill, verify position shown
4. Kill gateway process, verify UI shows "Exchange offline"
5. Restart gateway, verify auto-reconnect and data resumes
6. Load /docs through nginx, verify no 502
7. Direct port 49171: repeat steps 1-3 (no regression)
