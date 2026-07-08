# rsx-tui

The RSX trading terminal: a ratatui full-screen client that shows a
live orderbook ladder, order entry, positions, a trade tape, and a
latency breakdown, driven over a single gateway connection.

Transport is **protobuf-over-QUIC only** — one quinn connection, one
bidirectional stream, length-delimited protobuf frames. On connect the
client sends an **auth first-frame** (`WireHello`: an HS256 JWT + the
user id) before any order, so the session carries identity in-band; each
order frame then carries the `symbol` it trades and a client correlation
id echoed back on the latency report. There is no WebSocket path. The
internal casting (GW↔Risk↔ME) transport is unrelated and untouched; QUIC
here is the user-facing client↔gateway leg.

## Run

```bash
cargo run -p rsx-tui
```

With no gateway configured it runs an **offline demo** (a `MockConn`
seeded with a scripted book, trades, a position, and latency samples) so
a bare `rsx-tui` shows a live-looking screen with nothing running.

To dial a real QUIC gateway:

| Env | Meaning |
|-----|---------|
| *(default, no env)* | Dial **`rsx.krons.cx`**, the production server — trusts real CA roots, no cert file needed. Just run `rsx-tui`. |
| `RSX_GW_LOCAL` | Set → dial the local debug gateway `127.0.0.1:4433`; trusts the DER cert at `RSX_GW_CERT` (default `gateway.der`), TLS name `localhost`. |
| `RSX_GW_ADDR` | `mock` → offline demo. `ip:port` → an explicit pinned dial (needs `RSX_GW_CERT`). Unset → production. |
| `RSX_GW_CERT` | DER cert to trust for a local / explicit dial (not needed for production). |
| `RSX_GW_SERVER_NAME` | TLS name for an explicit dial (default `localhost`). |
| `RSX_TUI_USER` | User id (`u32`) the session trades as; minted into the auth-first-frame JWT's `user_id` claim (default `0`). |
| `RSX_TUI_SYMBOL` | Symbol id (`u32`) stamped on every order (default `0`). |
| `RSX_GW_JWT_SECRET` | HS256 secret used to sign the session JWT (dev default provided; the launcher holds the real one). |

No production gateway speaks this wire yet (see ARCHITECTURE.md
"Server side"), so the real dial is exercised against a loopback QUIC
server in `tests/quic_test.rs`. The gateway VALIDATING the auth
first-frame is the server-side follow-up; the client already sends it.

## Keys

- digits / `Backspace` — edit the focused order field
- `Tab` — move focus between price and qty
- `b` / `s` — set side Buy / Sell
- `t` — cycle time-in-force (GTC → IOC → FOK)
- `Enter` — submit the order
- `F3` — toggle the diagnostic trace overlay
- `q` / `Esc` — quit

## Access

Handing a session to a person (SSH forced-command dispatch, browser
terminal) is `specs/2/54-tui-access.md` — an access layer around this
binary, independent of its transport.
