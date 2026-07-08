# rsx-tui

The RSX trading terminal: a ratatui full-screen client that shows a
live orderbook ladder, order entry, positions, a trade tape, and a
latency breakdown, driven over a single gateway connection.

Transport is **protobuf-over-QUIC only** — one quinn connection, one
bidirectional stream, length-delimited protobuf frames (`OrderReq` out,
gateway events in). There is no WebSocket path. The internal casting
(GW↔Risk↔ME) transport is unrelated and untouched; QUIC here is the
user-facing client↔gateway leg.

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
| `RSX_GW_ADDR` | Gateway socket address (`ip:port`). Unset or `mock` → offline demo. |
| `RSX_GW_CERT` | Path to the gateway's DER certificate to trust. Required when `RSX_GW_ADDR` is set. |
| `RSX_GW_SERVER_NAME` | TLS name to validate (default `localhost`). |

No production gateway speaks this wire yet (see ARCHITECTURE.md
"Server side"), so the real dial is exercised against a loopback QUIC
server in `tests/quic_test.rs`.

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
