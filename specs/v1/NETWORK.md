# System Network Topology

## Overview

RSX uses a three-stage flow that separates ingress adaptation (gateway),
user-scoped risk, and symbol-scoped execution (matching engine). Each stage
is a monolithic process — no distributed consensus, no Raft, no cross-process
coordination within a tier.

```
External                Internal                        Execution
─────────────────────   ──────────────────────────────  ──────────────────
Web Users (WS)
                  ↘
Native Clients     ──→  Gateway (WS overlay)  ──→  Risk Engine  ──→  Matching Engine
(WebSocket)            (monolithic process)        (monolithic)     (one per symbol)
                  ↗                                                       |
Mobile Apps (WS)                                                     [SPSC events]
                                                                          |
                                                                     MARKETDATA
Web Users (WS) ──────────────────────────────────────────────────→  (public WS)
```

## Why This Topology

**Gateway before Risk Engine:**
- Gateway adapts web traffic to compact WebSocket protocol
- Risk engine remains user-scoped (positions, balances, margin)
- Separation keeps gateway lightweight and latency-focused
- Risk engine stays isolated from external traffic and parsing

**Matching Engine separate:**
- Symbol-scoped (one process per symbol or symbol group)
- Single-threaded per symbol (no locks, cache-friendly, reference ORDERBOOK.md)
- Stateless regarding users (just order IDs, no position tracking)
- Scales horizontally: add symbols by adding processes
- Clean isolation: BTC-PERP and ETH-PERP cannot interfere

## Component Architecture

### Gateway (Ingress Overlay)

**Responsibilities:**
- WebSocket overlay for web clients (see WEBPROTO.md)
- WebSocket for native clients
- User authentication and session management
- Rate limiting per user/IP
- Ingress backpressure and overload rejection (cap 10k orders)

**Architecture:**
- Monolithic process
- Async runtime (Tokio) for concurrent client sessions
- One WebSocket connection per client app
- QUIC connection to Risk Engine (multiplexed streams)
- Horizontal scaling: shard by user ID hash (load balancer routes by user_id)
- No cross-instance coordination (each instance owns its users)

### Risk Engine

**Responsibilities:**
- Position tracking (long/short qty per symbol)
- Margin calculation (initial margin, maintenance margin)
- Risk checks BEFORE sending orders to matching engine
- Fill ingestion and position update after matching

**Architecture:**
- Monolithic process
- QUIC connection from Gateway (multiplexed streams)
- QUIC connection to each Matching Engine (multiplexed streams)

**Design Note:**
Risk engine internals (margin models, position tracking, liquidation logic)
are NOT covered in this document. See future RISK.md for details. This doc
focuses on network topology and communication patterns.

### Matching Engine (One Per Symbol)

**Responsibilities:**
- Houses orderbook for one symbol (reference ORDERBOOK.md)
- Validates tick size, lot size (reference ORDERBOOK.md section 2)
- Single-threaded matching (cache-friendly, O(1) operations)
- Generates FILL events with balance/risk impact
- Stateless regarding users (no position tracking, no margin checks)
- Deduplicates orders via UUIDv7 tracking (reference RPC.md)

**Architecture:**
- Monolithic per symbol (NOT distributed across machines)
- Single-threaded event loop (no locks, no mutexes)
- Pre-allocated orderbook array (reference ORDERBOOK.md section 7)
- Event emission to Risk via QUIC streams

**Scaling:**
- Horizontal by symbol: one process per symbol or symbol group
- High-volume symbols get dedicated processes (BTC-PERP, ETH-PERP)
- Low-volume symbols can be grouped (all memecoins in one process)
- No cross-symbol coordination in v1

## Scaling Strategy

### Gateway: User Sharding

```
                   Load Balancer
                  (hash user_id)
                 /       |       \
         Gateway1    Gateway2    Gateway3
          users      users       users
          0-999      1000-1999   2000-2999
            ↓           ↓           ↓
         [all matching engines accessible via QUIC from all gateways]
```

**Why user sharding:**
- Each gateway instance owns a subset of users
- No cross-gateway coordination (user state is local)
- Load balancer routes by user_id hash (sticky sessions)
- Failures affect only that gateway's users

**Scaling constraints:**
- Each gateway connects to its Risk Engine
- Risk Engine connects to all active matching engines
- Streams are long-lived and multiplexed (no per-user streams)

### Matching Engine: Symbol Isolation

```
Gateway1 ────┬──→ Matching Engine (BTC-PERP)
Gateway2 ────┤
Gateway3 ────┘

Gateway1 ────┬──→ Matching Engine (ETH-PERP)
Gateway2 ────┤
Gateway3 ────┘

Gateway1 ────┬──→ Matching Engine (DOGE-PERP + SHIB-PERP)
Gateway2 ────┤   (low-volume symbols grouped)
Gateway3 ────┘
```

**Why symbol isolation:**
- No cross-symbol dependencies (BTC matching doesn't block ETH matching)
- Single-threaded per symbol (cache-friendly, no locks)
- Scale by adding symbols (not by distributing one symbol across machines)

**Scaling constraints:**
- One symbol = one process (no distributed orderbook in v1)
- High-throughput symbols need dedicated hardware
- Low-latency requires dedicated core pinning

## Communication Topology

### External → Gateway (Internet-Facing)

**WebSocket API (v1):**
- TLS encrypted
- Compact JSON frames (single-letter types, positional arrays)
- Authentication via JWT in headers
- Session management via WebSocket connection ID
- Protocol defined in WEBPROTO.md

### Internal: Gateway ↔ Risk ↔ Matching Engine

**Transport:**
- QRPC/UDP for live order/fill path (lowest latency)
- QRPC/QUIC for WAL replay and replication (reliable streaming)
- WAL stores fixed-record payloads (raw #[repr(C)] structs)
- Same wire format on disk, UDP, and QUIC — no transformation
- See QRPC.md for full transport specification

**Connection lifecycle:**
1. User opens WebSocket connection to Gateway
2. User sends order for BTC-PERP
3. Gateway forwards order over QUIC to Risk
4. Risk validates and forwards order over QUIC to Matching Engine
5. Matching engine processes, sends FILL messages back to Risk
6. Risk updates user positions, forwards fills to Gateway
7. Gateway forwards fills to user

**Backpressure layers (independent):**
- **Gateway ingress:** Gateway rejects new orders with
  `OVERLOADED` when its buffer exceeds capacity. This is
  the external-facing backpressure mechanism.
- **ME internal rings:** ME stalls on SPSC ring full (bare
  busy-spin). This is internal backpressure between
  co-located components. Gateway never sees this directly.
- These two layers are independent. Gateway rejection
  protects against external flood; ME stall protects against
  internal consumer lag.

**Stream semantics:**
- Long-lived (duration of process uptime)
- Bidirectional: Gateway → Risk → Matching (ORDER, CANCEL), reverse for FILL, ORDER_DONE
- Multiplexed by user_id and symbol (no per-user streams)
- Closed only on process shutdown or reconnect

**Transport:**
- v1: QRPC/UDP for live path, QRPC/QUIC for replay/replication
- See QRPC.md for full specification

**Replication transport:** ME and Risk replicas receive event
streams via DXS QRPC streaming (same WAL records over QUIC,
see QRPC.md and DXS.md section 5). No special replication
protocol — replicas are DXS consumers with the same
replay/live-tail mechanism used by all consumers.

## Data Flow

### Order Submission Flow

```
User ──ORDER──→ Gateway
                   │
                   ├─ Authenticate
                   ├─ Rate limit check
                   ├─ Ingress backpressure (fail fast)
                   │
                   ├─ Assign UUIDv7 order ID
                   ├─ Add to pending VecDeque
                   │
                   └──ORDER──→ Risk Engine
                                  │
                                  ├─ Margin check (risk)
                                  └──ORDER──→ Matching Engine
                                  │
                                  ├─ Validate tick/lot size
                                  ├─ Match against orderbook
                                  ├─ Generate FILL events
                                  │
                                  ├──FILL──→ Risk (0+ times)
                                  └──ORDER_DONE/FAILED──→ Risk
                                     │
                                     ├─ Pop from pending VecDeque
                                     ├─ Update user positions
                                     ├─ Recalculate margin
                                     │
                                     └──FILL/DONE──→ Gateway → User
```

See RPC.md for async request handling details.
See MESSAGES.md for message semantics (transport is now QUIC).

### Fill Notification Flow

```
Matching Engine
    │ (user A's order matches user B's order)
    │
    ├──FILL──→ Gateway1 (user A's gateway)
    │             ├─ Update user A position (+qty)
    │             └─ Forward FILL to user A
    │
    └──FILL──→ Gateway2 (user B's gateway)
                  ├─ Update user B position (-qty)
                  └─ Forward FILL to user B
```

**Key insight:** One fill in matching engine → two FILL messages sent (one to each user's gateway).

### Risk Update Flow

```
Risk receives FILL
  ↓
Update position (long_qty, short_qty)
  ↓
Recalculate margin (initial_margin, maintenance_margin)
  ↓
Check if margin < maintenance_margin
  ↓
If yes: trigger liquidation (send market orders to matching engine)
```

Risk checks happen BOTH:
- **Before order submission:** Ensure user has margin to open position
- **After fill:** Ensure position doesn't violate maintenance margin

## Network Boundaries

### External (Internet-Facing)

**Protocols:**
- WebSocket (compact JSON, WEBPROTO.md)
- QUIC (native clients, raw fixed records)

**Security:**
- TLS 1.3 encryption
- JWT authentication
- IP-based rate limiting
- DDoS protection (Cloudflare, load balancer)

**Transport:**
- TCP (internet → load balancer → gateway)
- TLS termination at load balancer OR gateway (config-dependent)

### Internal (Same Data Center)

**Same machine:**
- QUIC over localhost (quinn)
- TLS 1.3 built into QUIC
- Lowest latency (~50-100us, reference UDS.md)

**Cross-machine (same private VLAN):**
- QUIC over UDP (quinn)
- TLS 1.3 built into QUIC (no external IPsec)

**Cross-machine (untrusted network):**
- QUIC with mutual TLS (certificate validation)

## Performance Characteristics

### External → Gateway

**Latency:**
- Internet → TLS handshake: ~50-200ms (initial)
- JSON WebSocket message: ~1-10ms (after handshake)
- QUIC message: ~1-5ms (lower serialization overhead)

**Bottleneck:**
- Network latency (internet → data center)
- TLS encryption/decryption (mitigated by 0-RTT reconnect)

### Gateway → Risk → Matching Engine

**Latency (same machine, QRPC/UDP):**
- <10us per message
- Includes: sendto, fixed record memcpy, recvfrom

**Latency (same datacenter, QRPC/UDP):**
- <50us per message
- Includes: sendto, fixed record, network switch, recvfrom

**Future optimization:**
- No v2 planned (see FUTURE.md)

## Deployment Topologies

### Single Machine (Development, Small Deployment)

```
┌─────────────────────────────────────┐
│         Machine 1                    │
│                                      │
│  Gateway ──QUIC──→ Risk ──QUIC──→ Matching BTC  │
│                      │                        │
│                      └─────────QUIC──→ Matching ETH  │
└─────────────────────────────────────┘
```

**Benefits:**
- Lowest latency (QUIC localhost, no network)
- Simplest deployment (single binary, single config)

**Limits:**
- CPU cores (matching engines need dedicated cores)
- Memory (orderbook per symbol)

### Distributed (Production)

```
┌──────────────────┐       ┌──────────────────┐
│   Machine 1       │       │   Machine 2       │
│                   │       │                   │
│  Gateway1 ────────┼──QUIC─┼──→ Risk ──QUIC─→ Matching BTC   │
│  Gateway2 ────────┼──QUIC─┼──→ Risk ──QUIC─→ Matching ETH   │
└──────────────────┘       └──────────────────┘
       │                            │
       └──────────QUIC──────────────┘
          (all gateways talk to risk; risk talks to all matchers)
```

**Benefits:**
- Horizontal scaling (add gateway/matcher machines independently)
- Isolation (matching engine failures don't kill gateways)
- Dedicated hardware per component (gateways need network I/O, matchers need CPU/cache)

**Complexity:**
- Network configuration (private VLAN, routing)
- Service discovery (how risk finds matching engines)
- Failure handling (stream reconnection, order replay)

## Service Discovery

### v1: Environment Variables

Static configuration via environment variables with component
prefix:

```
RSX_ME_BTC_ADDR=127.0.0.1:9001
RSX_ME_ETH_ADDR=127.0.0.1:9002
RSX_RISK_ADDR=127.0.0.1:9010
RSX_GATEWAY_ADDR=0.0.0.0:8080
RSX_MARK_ADDR=127.0.0.1:9200
RSX_POSTGRES_URL=postgres://localhost:5432/rsx
```

Each component reads its own address and the addresses of its
dependencies from environment variables. No service registry
for v1.

### Dynamic Discovery (Optional)

For deployments with more than a handful of components, optional
Consul or DNS-based discovery can replace env vars. This is
independent of the static config mechanism — components check
env vars first, fall back to Consul/DNS if configured.

Discovery and static config are independent mechanisms. They
do not interact or override each other.

## Startup Ordering

Components can start in any order. Each component retries
connecting to its dependencies with exponential backoff
(1s/2s/4s/8s, max 30s).

- **Matching engine:** starts independently, begins WAL
  replay. Serves DXS replay once ready.
- **Risk engine:** retries ME connections with backoff.
  Replays from DXS on each successful connect.
- **Gateway:** retries Risk connection with backoff. Rejects
  all user orders with `OVERLOADED` until Risk stream is
  established and Risk reports ready (CaughtUp on all
  streams).
- **Mark aggregator:** starts independently, connects to
  external feeds with backoff.

No component blocks on another's availability. The system
converges to ready state as components come online.

## Failure Modes

### Gateway Failure

**Impact:**
- Users connected to that gateway lose connection
- Orders in flight on that gateway are lost (user must retry)
- Other gateways unaffected

**Recovery:**
- Load balancer removes failed gateway from pool
- Users reconnect to healthy gateway
- User state (positions, balances) recovered from database

**No order replay:**
- Gateway crash = orders lost (user must resubmit)
- Simpler than distributed transaction log
- Acceptable for v1 (users can retry)

### Risk Engine Failure

**Impact:**
- Gateways cannot submit orders
- Matching engines continue running but receive no new orders
- Users see errors and must retry after recovery

**Recovery:**
- Restart risk engine process
- Gateways reconnect the single stream
- Order flow resumes

### Matching Engine Failure

**Impact:**
- Symbol becomes untradable (e.g., BTC-PERP down)
- Risk loses stream to that matching engine
- Other symbols unaffected (ETH-PERP still works)

**Recovery:**
- Restart matching engine process
- Rebuild orderbook from snapshot + WAL
- Risk reconnects stream
- Users can submit new orders

**Orderbook persistence:**
- Append-only WAL (every order, cancel, fill)
- Online snapshot periodically (reduce replay time)
- Replay WAL after snapshot on startup

### Network Partition

**Gateway ↔ Risk partition:**
- Gateway cannot send orders to risk
- Gateway rejects orders at ingress when buffer is full

**Risk ↔ Matching Engine partition:**
- Risk cannot send orders to matcher
- Risk returns error to gateway
- Mitigation: UUIDv7 deduplication in matching engine
  (reference MESSAGES.md for semantics)

### MARKETDATA (Public Market Data)

**Responsibilities:**
- Maintains shadow orderbook per symbol (shared `rsx-book` crate)
- Derives L2 depth, BBO, and trades from ME events
- Serves public WebSocket endpoint for market data subscriptions
- Recovery via DXS replay from ME WAL

**Architecture:**
- Single-threaded, dedicated core, busy-spin
- Non-blocking epoll for WS I/O (no Tokio)
- One SPSC consumer ring per matching engine
- Separate process from gateway (public, no auth)

See [MARKETDATA.md](MARKETDATA.md) for full specification.

## Cross-References

- **ORDERBOOK.md**: Matching engine internals, orderbook data structure
- **MARKETDATA.md**: Market data dissemination, shadow orderbook, L2/BBO/trades
- **SMRB.md**: Low-latency IPC options, SPSC ring buffer design
- **UDS.md**: UDS vs shared memory comparison, latency numbers
- **RPC.md**: Async request handling, pending order tracking
- **MESSAGES.md**: Message semantics (transport is now QUIC)
- **WEBPROTO.md**: WebSocket overlay and compact wire protocol
