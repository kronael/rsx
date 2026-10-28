# rsx-ws Architecture

Three small modules over monoio's `TcpStream`; the gateway and marketdata call
them from their connection handlers. Nothing here spawns a task, holds state
or touches a record type.

| File | Purpose |
|------|---------|
| `lib.rs` | Module list and re-exports |
| `accept.rs` | `ws_accept_loop`: bind, `TCP_NODELAY`, hand each connection and its peer address to the caller |
| `upgrade.rs` | `WS_UPGRADE_DEADLINE`, `read_http_request` (8 KiB cap, `431`, deadline), `header_value`, `is_ws_upgrade`, the accept key and the `101` reply, `ws_upgrade` |
| `frame.rs` | `ws_read_frame_buf` (mask required, FIN=1, 4 KiB cap) and the frame writers |

```text
accept loop ─► handler(stream, peer)
                 │
                 ├─ read_http_request ── 431 / TimedOut ─► close
                 │
                 ├─ gateway:    JWT check ─ 401 ─► close
                 │                 └─► write_upgrade_response
                 └─ marketdata: ws_upgrade (101)
                 │
                 └─ readable ─► ws_read_frame_buf(leftover) ─► ws_write_*
```

## Request read

The head is read in 4096-byte chunks until the blank line. A head whose blank
line ends past 8192 bytes, or that has no blank line by 8192 bytes, gets `431`
and an `InvalidData` error. One `monoio::time::timeout` bounds the whole read,
so a client that dribbles bytes cannot hold the connection past
`WS_UPGRADE_DEADLINE`. Bytes read past the blank line are returned as
`leftover` and are the first input of `ws_read_frame_buf`.

## Frame reader

`ws_read_frame_buf` takes from `buf` before it reads the stream. Dropping its
future mid-frame loses the bytes already taken, so a handler waits for
`stream.readable` first and then runs the reader to the end. A 64-bit length
field is read and then refused by the 4 KiB cap.

## Writers

Every writer builds the frame in one `Vec` and issues one `write_all`. Server
frames are never masked or fragmented.
