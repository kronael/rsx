# TUI QUIC Protobuf

`rsx-tui` uses protobuf-over-QUIC for the user-facing
client edge. The tradeoff is clear: a native terminal gets the
fast binary path; browsers need a WebTransport-over-HTTP/3
front door because they cannot open raw QUIC sockets.

## Why QUIC at the edge

QUIC gives the terminal a single encrypted connection with
stream transport, 0/1-RTT setup, connection migration, and no
TCP head-of-line blocking across independent streams. It also
has unreliable datagrams for feed data that can be dropped
instead of delaying newer book updates.

That matches the exchange shape. Internal RSX traffic uses
casting: one WAL record per UDP datagram, NAK recovery, no
broker. The terminal is outside that trust boundary, so it does
not receive raw `repr(C)` WAL records, but the rule stays the
same: bytes go directly between the two endpoints. No message
broker, no HTTP request per action, no text envelope on the
order path.

## Why protobuf

The live `rsx-tui` wire is one QUIC connection, one
bidirectional stream, and length-delimited protobuf frames:
a 4-byte big-endian length prefix, then a `prost` body.
The first client frame is `WireHello { jwt, user }`; every
order frame carries a `cid`, `symbol`, side, price, qty, and
TIF. The frame cap is 1 MiB, far above the legitimate order
and event frames.

Protobuf is slower and larger than RSX's internal fixed records,
but it is still binary, compact, and fast to parse compared with
JSON. It gives the external edge typed fields and schema
evolution without paying the JSON parse cost on every order.
That is the same-bytes philosophy adapted to an untrusted
client boundary rather than copied blindly across it.

## Where it scales

The terminal client is proven against a loopback `quinn` server,
not load-tested as a gateway service. The scale path is the
same io_uring ladder used for gateway and marketdata: multishot
recv, registered buffers, then SQPOLL when the deployment gives
the process a dedicated core. SQPOLL is gated by that dedicated-
core configuration because it burns a polling kernel thread to
remove the submit syscall.

The tradeoff is operational, not conceptual. A gateway QUIC
server does not exist yet; the crate is the client half and the
loopback test is the executable contract. Gateway-side
validation of the `WireHello` JWT is still pending. Until that
listener exists, the public gateway path remains WebSocket JSON
and the terminal's real QUIC dial is a client-side proof.

---

Deeper: [specs/2/54-tui-access.md](../../specs/2/54-tui-access.md),
[specs/2/55-terminal.md](../../specs/2/55-terminal.md),
[specs/2/49-webproto.md](../../specs/2/49-webproto.md),
[specs/2/11-gateway.md](../../specs/2/11-gateway.md),
[docs/concepts/reliable-udp.md](../../docs/concepts/reliable-udp.md),
[docs/concepts/wal-is-wire-is-stream.md](../../docs/concepts/wal-is-wire-is-stream.md),
[blog/picking-a-wire-format.md](../../blog/picking-a-wire-format.md),
[blog/casting.md](../../blog/casting.md)
