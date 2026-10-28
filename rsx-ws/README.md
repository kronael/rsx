# rsx-ws

The monoio WebSocket core the gateway and marketdata share: the accept loop,
the bounded upgrade read, the `101` reply and the frame reader and writers.
It knows no JWT and no record type; the callers add those. Not published
(`publish = false`).

## What it provides

- `ws_accept_loop(addr, handler)`: binds `addr`, sets `TCP_NODELAY` and calls
  `handler(TcpStream, SocketAddr)` for every connection.
- `read_http_request(stream)`: the request head and the bytes past it. A head
  over 8 KiB is answered `431` and refused with `InvalidData`; a head not
  complete within `WS_UPGRADE_DEADLINE` (5 s) is a `TimedOut` error.
- `is_ws_upgrade`, `header_value`: request inspection.
- `write_upgrade_response(stream, key)`: the `101` with the accept key.
- `ws_upgrade(stream)`: `read_http_request` plus the `101`, with no
  authentication; returns the bytes past the head.
- `ws_read_frame_buf(stream, buf)`: one client frame, `buf` consumed first.
  The frame must be masked, FIN=1 and at most 4096 bytes.
- `ws_write_text`, `ws_write_binary`, `ws_write_frame(opcode)`: unmasked FIN
  frames.

## Running the tests

```text
cargo test -p rsx-ws
```

Unit tests sit in `src/*_test.rs` and run over loopback sockets.
