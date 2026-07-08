# The Trading Terminal

`rsx-tui` is a keyboard-driven ratatui terminal for one instrument.
Two design choices define it: the fastest client transport, and the
highest-bandwidth input model — protobuf-over-QUIC in, keys out.

## Transport: protobuf over QUIC

The terminal dials the exchange over one QUIC connection carrying
protobuf frames. QUIC removes TCP head-of-line blocking across
independent streams, sets up in 0–1 RTT, migrates across network
changes, and offers unreliable datagrams for feed data that can be
dropped rather than delay newer book updates. Protobuf is binary and
compact — typed fields and schema evolution without paying the JSON
parse cost on every order.

It extends the same-bytes, no-broker philosophy to the client edge,
but adapted for an *untrusted* boundary: the terminal is outside the
casting trust zone, so it does not receive raw `repr(C)` WAL records —
it gets a typed protobuf wire. The frame is a 4-byte big-endian
length prefix then a `prost` body; the first client frame is a
`WireHello { jwt, user }` auth handshake, and every order carries a
`cid`, `symbol`, side, price, qty, and TIF.

The honest tradeoffs: no gateway QUIC server answers this wire yet —
the client half is proven against a loopback `quinn` server, and
gateway-side validation of the `WireHello` JWT is the pending
roadmap step. Browsers can't open raw QUIC; a browser client would
need WebTransport over HTTP/3.

## Input: keyboard, not mouse

Order entry is an input-bandwidth problem. A keyboard turns actions
into home-row chords; a mouse turns them into serial point-and-click
targeting, one target at a time. Fitts's law is the cost model:
click time rises with target distance and falls with target size, so
a dense trading GUI is slowest exactly where it matters most — small
adjacent controls under time pressure.

Today nine action classes are bound — digits edit price and qty,
`Tab` changes field, `b`/`s` choose side, `t` cycles TIF, `Enter`
submits, `F3` toggles diagnostics, `q`/`Esc` exits — all reachable
without leaving the keys. But nine is what is *bound*, not the
ceiling: the input space is the whole keyboard, plus modifiers and
modal layers. Binding it out fully is deliberate future design; the
point is that the room to grow is enormous and free, where a mouse
GUI needs more screen and more targets for every new action.

The prior art isn't decorative — Bloomberg terminals and vim/modal
editors win by making repeated actions addressable from keys, so
execution becomes recall instead of search. The tradeoff is
discoverability: keyboard-first UIs are less guessable, and modal
mistakes cost on a trading screen. RSX keeps an always-visible
key-hint bar, so as the command set grows from nine toward the full
keyboard, nothing stays hidden.

---

Deeper: [specs/2/54-tui-access.md](../../specs/2/54-tui-access.md),
[specs/2/55-terminal.md](../../specs/2/55-terminal.md),
[specs/2/49-webproto.md](../../specs/2/49-webproto.md),
[specs/2/11-gateway.md](../../specs/2/11-gateway.md),
[blog/25-trade-ui-notes.md](../../blog/25-trade-ui-notes.md)
