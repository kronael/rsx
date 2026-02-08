# RSX Design Critique (as of 2026-02-08)

This critique covers the current design docs in this repo: `NETWORK.md`, `RPC.md`, `PROTOCOL.md`, `CONSISTENCY.md`, `SMRB.md`, `UDS.md`, `ORDERBOOK.md`, `ORDERBOOKv2.md`, `ARENA.md`, `HOTCOLD.md`, `ALIGN.md`, and the wire-format blog posts. It is intentionally comprehensive and grounded in external references listed at the end.

## Executive Summary

The design shows clear low-latency intent and a coherent decomposition between user-scoped and symbol-scoped concerns. The move from gRPC/protobuf to shared-memory rings is sensible for latency, and the orderbook compression + recentering idea is thoughtful for bounded memory. However, the current plan carries major correctness and operational risks: recovery semantics are underspecified (the matching engine is ephemeral), price-time priority can be violated in compressed buckets, and the gateway↔matcher stream model scales poorly. There are also protocol and transport details that will materially affect tail latency and failure behavior (HTTP/2 flow control, gRPC keepalives, browser gRPC-web limitations). Addressing these now will prevent architectural lock-in.

## What Is Strong

- Clear separation of user-state (gateway/risk) and symbol-state (matching engine) keeps hot paths single-threaded and cache friendly.
- Explicitly fixed-point pricing avoids float drift at the API boundary and keeps matching deterministic.
- The documented future path from gRPC to shared memory demonstrates awareness of the real latency bottlenecks.
- The compression + lazy recentering idea is a serious attempt to bound memory while supporting large price ranges.

## Critical Gaps and Risks

1. **Recovery semantics are incomplete**. Matching engines are “ephemeral” and on crash the book is empty. That makes order-state and price-time priority effectively non-recoverable unless you have a durable order log or snapshot at the matcher. Risk-engine persistence alone is not sufficient to reconstruct the exact matching state. This should be a top priority because it affects market integrity and replay semantics.

2. **Price-time priority can be violated in compressed slots**. The `ORDERBOOK.md` design compresses multiple prices into a single slot and then walks linked lists in insertion order. Unless those lists are sorted by price, you can match a worse price before a better price within the same bucket. Most exchange rulebooks explicitly require price priority before time priority, which means the matching loop must either sort by price within a bucket or maintain per-price queues even inside compressed zones. See the NASDAQ price/time execution rules as an example of required ordering semantics.

3. **Gateway↔matcher streams likely explode at scale**. The “one stream per user per symbol” model creates O(users × symbols) long-lived streams. Even with HTTP/2 multiplexing, a large number of streams increases memory, per-connection state, and flow-control complexity. A more scalable model is to multiplex many users over a small number of streams per gateway↔matcher pair and build a lightweight application-level routing layer.

4. **HTTP/2 flow control and gRPC keepalive are not accounted for**. HTTP/2 has mandatory flow control and per-stream windows; gRPC adds its own keepalive requirements. Long-lived, high-volume streams can stall if windows are mis-sized or if keepalive policies conflict with server settings. This can create silent backpressure that looks like matching latency.

5. **Browser gRPC-web limitations**. gRPC-web cannot fully support bi-directional streaming in today’s browsers without proxies and has feature limits. The “web client” path should not assume parity with native gRPC.

6. **Backpressure in event fan-out can stall matching**. The current SPSC fan-out uses busy-spin on full rings. Any slow consumer (market data or gateway) can stall the matching engine. This is a deliberate design choice, but it needs an explicit overload policy: drop/non-blocking for market data, dedicated buffering, or fail-closed behavior, and this should be spelled out as a tradeoff.

7. **Transport security and trust boundaries are not precise**. The docs indicate “no TLS on private VLAN” for v1, but do not state how endpoint identity is verified or how to detect misconfiguration. If you operate in multi-tenant or shared environments, you will want an explicit network authentication story (mTLS or network-level security).

## Detailed Critique by Area

### 1) Network Topology and Scaling (`NETWORK.md`)

- **Stream explosion risk**. A per-(user, symbol) stream model is very costly. A gateway with 100k users and 100 symbols could imply millions of active streams. HTTP/2 multiplexing helps with a single connection, but each stream still has flow-control state and gRPC metadata overhead. Consider a per-symbol multiplexed stream, or a per-gateway stream per matcher with user_id routing in messages.
- **HTTP/2 flow control and head-of-line**. HTTP/2 uses per-connection and per-stream flow control, and only DATA frames are flow-controlled. Large bursts in one stream can stall another if windows aren’t managed correctly. You should define window sizing and load-shedding strategy explicitly.
- **Keepalive for long-lived connections**. gRPC relies on HTTP/2 PING for keepalive; servers can reject aggressive keepalive policies. Make the keepalive policy explicit and consistent across gateways and matchers.
- **Web client assumptions**. gRPC-web does not fully support bi-di streaming in browsers without proxies and has known limitations; if a browser client is in scope, the protocol must account for this.

### 2) RPC and IDs (`RPC.md`)

- **UUIDv7 time ordering depends on clock discipline**. UUIDv7 embeds a millisecond timestamp. If gateway clocks drift or leap, ordering in logs and idempotency windows can become inconsistent. You should explicitly require clock sync (NTP/chrony) and define how to handle time regressions.
- **Retry/ambiguity policy**. The “no automatic retry” rule keeps complexity low, but it does not resolve ambiguity during partial failures (e.g., gateway crash after sending but before ACK). This interacts with client-side retry semantics and dedup windows. Consider returning explicit “unknown” outcomes and a query API for order status.

### 3) Protocol and Serialization (`PROTOCOL.md`)

- **Wire type costs**. Protobuf uses varint encoding for many numeric types and fixed-width wire types for others. If your `int64` fields are always non-negative and fixed width, you may want `uint64` or `fixed64` to avoid varint overhead and sign-extension edge cases. This matters on the hot path.
- **Versioning**. The protocol does not include explicit versioning or capability negotiation. This will be essential if you migrate to raw structs or change match semantics.
- **Message limits**. Protobuf has a 2 GiB message size limit when serialized; if you plan to batch fills or snapshots, that limit matters.

### 4) Consistency and Persistence (`CONSISTENCY.md`)

- **Ephemeral matcher is a correctness risk**. If you lose the book, you lose precise price-time priority and may execute future matches in a different order than the pre-crash state. This conflicts with rulebook expectations on ordering and time priority in many markets. Even a minimal WAL or snapshot at the matcher would dramatically improve correctness.
- **Fan-out backpressure**. The “ring full = stall matching” policy is valid for correctness, but it should be framed as a reliability tradeoff with explicit overload behavior. At minimum, market-data fan-out could be lossy without stalling matching.
- **No cross-symbol ordering**. This is fine, but if you ever plan to add portfolio margin or cross-symbol risk, you’ll need a shared ordering or a dedicated risk synchronizer.

### 5) IPC: UDS vs Shared Memory (`SMRB.md`, `UDS.md`)

- **Socket type and framing**. UDS supports SOCK_STREAM and SOCK_SEQPACKET; only SOCK_SEQPACKET preserves message boundaries. If you ever drop protobuf framing or use a custom protocol over UDS, this choice becomes critical.
- **Shared memory correctness**. With `shm_open`/`mmap`, you own synchronization, memory ordering, and crash semantics. The docs should spell out the memory model and include a correctness checklist for crash recovery.
- **Security boundary**. UDS relies on filesystem permissions and can pass credentials; shared memory uses POSIX objects with their own permissions. This should be explicitly documented for deployments with multiple tenants or untrusted local code.

### 6) Orderbook and Matching (`ORDERBOOK.md`, `ORDERBOOKv2.md`)

- **Price-time priority within compressed slots**. The current design preserves time priority but not necessarily price priority inside a “smooshed” bucket. If better prices and worse prices share a slot, a FIFO list can violate price priority unless you add per-price queues or a price-ordered structure within the slot. This is the biggest matching correctness risk.
- **Worst-case latency under extreme moves**. Smooshed zones imply linear scans; in tail events you can accumulate many orders in the same bucket and do O(k) scans on the hot path. You need hard worst-case limits or an admission control policy.
- **Recenter complexity**. Lazy migration interleaves with matching, but it still adds unpredictable latency spikes. Define and test explicit max migration work per match cycle.
- **Order types and controls**. v1 is GTC-only with no market orders, IOC/FOK, post-only, self-trade prevention, or auction handling. That’s fine for a narrow product, but if this is a broader exchange design it is a significant feature gap.

### 7) Memory and Layout (`ARENA.md`, `HOTCOLD.md`, `ALIGN.md`)

- **Alignment assumptions**. The design assumes 64-byte cache lines and uses `align(64)`; this is a reasonable default on modern x86, but should be stated as an assumption with guidance for verification on target hardware.
- **Arenas in latency-sensitive paths**. Arenas are fast and locality-friendly, but you need a clear lifetime model to avoid memory bloat during sustained high-load bursts. Consider a reset cadence or per-request arenas rather than long-lived ones.
- **Hot/cold splitting**. This is generally good, but it needs measurement. If hot/cold data is accessed together more than expected, the split can add pointer chasing and branch misses.

## Recommendations (Prioritized)

1. **Define a matcher persistence model**: minimal WAL + periodic snapshot, with deterministic recovery rules. This is the highest-impact risk reduction.
2. **Guarantee price-time priority in compressed buckets**: either per-price queues inside a bucket or a price-ordered structure, plus tests that enforce priority. Align with public exchange rulebook semantics.
3. **Reduce stream cardinality**: use gateway↔matcher streams that multiplex users, with application-level routing and backpressure control.
4. **Document flow-control and keepalive settings**: include HTTP/2 window sizing and gRPC keepalive policies for long-lived streams.
5. **Explicit web-client strategy**: if browser clients are real, adopt gRPC-web constraints and proxy architecture now.
6. **Specify backpressure policy**: decide where you can drop or shed load (market data vs risk vs gateway) and how you preserve correctness under overload.
7. **Clarify hardware assumptions**: cache line size, NUMA strategy, and core pinning should be explicit for reproducibility.

## References

- RFC 9562: UUID Version 7 specification (IETF). https://datatracker.ietf.org/doc/html/rfc9562
- gRPC Core Concepts: streaming semantics and message ordering. https://grpc.io/docs/what-is-grpc/core-concepts/
- gRPC About: HTTP/2 transport and bi-di streaming. https://grpc.io/about/
- HTTP/2 (RFC 7540): multiplexing and flow control. https://datatracker.ietf.org/doc/html/rfc7540
- TLS 1.3 (RFC 8446): security properties for encrypted transport. https://datatracker.ietf.org/doc/rfc8446/
- Protocol Buffers encoding: wire types, varint, fixed32/fixed64, size limits. https://protobuf.dev/programming-guides/encoding/
- Rust Reference: `repr(C)` and `align` modifiers. https://doc.rust-lang.org/stable/reference/type-layout.html
- UNIX domain sockets (`unix(7)`): AF_UNIX, socket types, message boundaries. https://www.man7.org/linux/man-pages/man7/unix.7.html
- POSIX shared memory: `shm_open`, `mmap` overview. https://man7.org/linux/man-pages/man3/shm_open.3p.html and https://man7.org/linux/man-pages/man2/mmap.2.html
- LMAX Disruptor user guide: ring buffer and sequencing concepts. https://lmax-exchange.github.io/disruptor/user-guide/
- NASDAQ price/time priority (example rulebook). https://listingcenter.nasdaq.com/rulebook/nasdaq/rules/Nasdaq%20Equity%204
- gRPC-Web limitations in browsers. https://grpc.io/blog/state-of-grpc-web/
- gRPC keepalive (HTTP/2 PING behavior). https://grpc.io/docs/guides/keepalive/
