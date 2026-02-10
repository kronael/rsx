# CMP: why we dropped gRPC for C structs over UDP

We had gRPC. It worked. We dropped it.

## the problem

Our exchange has one wire format: fixed-size `#[repr(C,
align(64))]` structs with a 16-byte header. Same bytes on
disk (WAL), in memory (SPSC rings), and over the network.
No serialization, no transformation, no copies.

Except for one place: DXS streaming. There we wrapped those
same bytes in protobuf, inside HTTP/2 frames, inside TCP.
Three layers of framing for bytes that were already framed.

```
disk:   [16B header][64B payload]
ring:   [16B header][64B payload]
gRPC:   [protobuf([16B header][64B payload])]
         inside HTTP/2 DATA frame
         inside TCP segment
```

That's protobuf encoding (~200ns) + HTTP/2 framing + TCP
head-of-line blocking. For bytes that need zero processing.

## the fix

We designed CMP: C Message Protocol. Two transports for two
paths, inspired by Aeron's reliability model.

**Hot path (CMP/UDP):** One WAL record per UDP datagram.
Aeron-style NACK + flow control for reliability. Sender
sends Heartbeats, receiver sends StatusMessages and Naks.
Retransmits are just normal records re-read from WAL.

**Cold path (WAL replication over TCP):** Plain TCP byte
stream. `write_all(header)`, `write_all(payload)`, repeat.
Optional TLS via rustls. No protocol name — just streaming
WAL bytes over a reliable transport.

```
CMP/UDP:  [16B header][64B payload]
           inside UDP datagram

WAL/TCP:  [16B header][64B payload]
           inside TCP stream
```

No existing Rust crate fits. Aeron is JVM-only. Everything
else (quinn, h3, tonic) adds redundant framing on top of
our already-framed WAL records. We took Aeron's NACK +
StatusMessage + flow control model and built it ourselves.

## why not QUIC for the cold path

We started with QUIC (quinn). Three problems:

1. **Mandatory TLS.** QUIC requires TLS 1.3. Can't disable
   it. For localhost WAL replay this is pure overhead.
2. **Redundant framing.** QUIC has its own framing on top
   of our WAL header. Two layers of length-prefixed framing.
3. **Complexity.** quinn pulls in a significant dependency
   tree for features we don't use (0-RTT, migration,
   multiplexing).

TCP gives us: reliable byte stream. That's all we need for
the cold path. Optional TLS via rustls when crossing
networks. No mandatory crypto for localhost.

## what we gained

**Latency:** ~200ns to encode a message (it's a memcpy)
vs ~2-5us for protobuf + gRPC framing.

**Simplicity:** One wire format everywhere. The function
that writes to WAL, the function that sends over the
network, and the function that pushes to an SPSC ring all
take the same bytes. No SerDe traits, no .proto files, no
codegen step.

**Reliability without TCP:** Aeron's NACK model gives us
gap detection and retransmission on UDP without TCP's
head-of-line blocking. Receiver detects gaps via Heartbeat,
sends Nak, sender re-reads from WAL. New records keep
flowing — retransmits are parallel, not blocking.

**Flow control without QUIC:** StatusMessage from receiver
tells sender how much it can accept. Sender won't exceed
`consumption_seq + receiver_window`. Natural backpressure
without transport-level flow control.

**Consistency:** When we debug, the bytes in tcpdump are
the same bytes in the WAL file are the same bytes in the
ring buffer. One hex dump tool works everywhere.

## what we gave up

These are real trade-offs. We accept all of them.

**1. No schema evolution.** Can't add a field to FillRecord
without breaking every reader. New features use new record
types. Breaking changes bump the version in the header and
require coordinated deployment. This is fine — all
components are compiled from the same repo.

**2. No cross-language support.** The wire format is
`#[repr(C)]` Rust structs. A Python client would need to
manually define the same struct layout with ctypes. But
external clients use the WebSocket JSON API (WEBPROTO.md),
not CMP. Internal is Rust-only.

**3. No human readability.** Can't `curl` the endpoint.
Can't read the wire with `jq`. We have a WAL dump tool
and structured tracing at every boundary. In practice we
never debug by reading raw wire bytes.

**4. Endianness lock-in.** Little-endian only. We're x86.
ARM aarch64 is also little-endian by default. If we ever
need big-endian (we won't), it's a rewrite.

**5. Custom reliability layer.** We maintain our own NACK +
flow control instead of using TCP or QUIC. More code to
write and test. But the model is proven (Aeron has run in
production at major exchanges for years) and the
implementation is small (~500 lines for sender + receiver).

## the 11 pitfalls

| # | Pitfall | Mitigation |
|---|---------|------------|
| 1 | Endianness | x86-only, compile-time assert |
| 2 | Alignment/padding | repr(C), explicit _pad, size asserts |
| 3 | No versioning | version in header, additive types |
| 4 | Torn reads | CRC32 validation, TCP reliability |
| 5 | Transmute UB | ptr::read on Copy types, no transmute |
| 6 | Invalid enums | raw integers on wire, validate at boundary |
| 7 | Float NaN | i64 fixed-point, zero floats |
| 8 | DoS via length | MAX_PAYLOAD = 64KB cap |
| 9 | No framing | 16B header with length field |
| 10 | No cross-language | Rust-only internal, JSON for external |
| 11 | No human readability | WAL dump tool, structured tracing |

Every one of these is documented, mitigated, and accepted.

## when you should NOT do this

- Multi-language teams (use protobuf or FlatBuffers)
- Public APIs (use gRPC or REST)
- Schema that changes frequently (use protobuf)
- Systems that need to run on arbitrary hardware (endianness)
- Teams that debug by reading wire captures (use JSON)

CMP is for: single-team, single-language, latency-sensitive
internal systems where you control every endpoint. That's an
exchange.

## the punchline

gRPC is a protocol for calling functions on remote machines.
We don't call functions. We stream bytes. The bytes are
already framed. We just need two things: low-latency
delivery (UDP + NACK) and reliable replay (TCP).

We just send the bytes.
