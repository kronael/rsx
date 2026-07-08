# TUI QUIC Protobuf

`rsx-tui` uses protobuf-over-QUIC for the user-facing client
edge. The tradeoff is explicit: a native terminal can use a
binary path, while browsers need WebTransport over HTTP/3
because they cannot open raw QUIC sockets.

## Why QUIC at the edge

QUIC gives the terminal one encrypted connection with streams,
0/1-RTT setup, connection migration, and no TCP head-of-line
blocking across independent streams. It also has unreliable
datagrams for feed data that can be dropped instead of delaying
newer book updates.

That matches the exchange shape. Internal RSX traffic uses
casting: one WAL record per UDP datagram, NAK recovery, no
broker. The terminal is outside that trust boundary, so it does
not receive raw `repr(C)` WAL records, but the rule stays close:
bytes go directly between 2 endpoints. No message broker, no
HTTP request per action, no text envelope on the order path.

## Why protobuf

The live `rsx-tui` wire is 1 QUIC connection, 1 bidirectional
stream, and length-delimited protobuf frames: a 4-byte big-endian
length prefix, then a `prost` body. The first client frame is
`WireHello { jwt, user }`; every order frame carries a `cid`,
`symbol`, side, price, qty, and TIF. The frame cap is 1 MiB, far
above legitimate order and event frames.

Protobuf is slower and larger than RSX's internal fixed records,
but it is still binary and typed. It gives the external edge schema
evolution without paying a JSON text parse on every order. That is
the same-bytes philosophy adapted to an untrusted client boundary
instead of copied blindly across it.

## What is not done

The gateway QUIC server does not exist yet. `rsx-tui` is the client
half plus a loopback `quinn` executable contract. Browser clients
need WebTransport over HTTP/3, not raw QUIC. Gateway validation of
the first-frame JWT is still pending.

Until that listener exists, the public gateway path remains
WebSocket JSON. The terminal QUIC path proves the client transport
shape; server-side throughput belongs to gateway and marketdata,
not this single-client frontend.

---

Deeper: [specs/2/54-tui-access.md](../../specs/2/54-tui-access.md),
[specs/2/55-terminal.md](../../specs/2/55-terminal.md),
[specs/2/49-webproto.md](../../specs/2/49-webproto.md),
[specs/2/11-gateway.md](../../specs/2/11-gateway.md),
[docs/concepts/network-edge-io.md](../../docs/concepts/network-edge-io.md),
[docs/concepts/reliable-udp.md](../../docs/concepts/reliable-udp.md),
[docs/concepts/wal-is-wire-is-stream.md](../../docs/concepts/wal-is-wire-is-stream.md),
[blog/picking-a-wire-format.md](../../blog/picking-a-wire-format.md),
[blog/casting.md](../../blog/casting.md)
