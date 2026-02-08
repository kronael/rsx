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
(gRPC)                 (monolithic process)        (monolithic)     (one per symbol)
                  ↗                                                       |
Mobile Apps (gRPC)                                                   [SPSC events]
                                                                          |
                                                                     MARKETDATA
Web Users (WS) ──────────────────────────────────────────────────→  (public WS)
```

## Why This Topology

**Gateway before Risk Engine:**
- Gateway adapts web traffic to a compact WebSocket protocol
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
- gRPC passthrough for native clients
- User authentication and session management
- Rate limiting per user/IP
- Ingress backpressure and overload rejection (cap 10k orders)

**Architecture:**
- Monolithic process
- Async runtime (Tokio) for concurrent client sessions
- One WebSocket connection per client app
- Single multiplexed gRPC stream to Risk Engine
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
- Single multiplexed gRPC stream from Gateway
- Single multiplexed gRPC stream to each Matching Engine

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
- Deduplicates orders via UUIDv7 tracking (reference RPC.md, PROTOCOL.md)

**Architecture:**
- Monolithic per symbol (NOT distributed across machines)
- Single-threaded event loop (no locks, no mutexes)
- Pre-allocated orderbook array (reference ORDERBOOK.md section 7)
- Event emission to Risk via bidirectional gRPC stream

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
         [all matching engines accessible from all gateways]
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

**gRPC (v1 implementation):**
- TLS encrypted
- gRPC streaming (bidirectional)
- Authentication via gRPC metadata (JWT)
- Native clients (desktop, mobile, trading bots)
- Protocol defined in PROTOCOL.md

### Internal: Gateway ↔ Risk ↔ Matching Engine

**Transport:**
- gRPC bidirectional streaming (v1, inter-process/network)
- SPSC rings are used for *in-process* tile communication
- WAL stores protobuf payloads only (no gRPC envelope)
- One multiplexed stream Gateway ↔ Risk
- One multiplexed stream Risk ↔ Matching Engine (per matcher)

**Connection lifecycle:**
1. User opens WebSocket/gRPC connection to Gateway
2. User sends order for BTC-PERP
3. Gateway forwards order over its single stream to Risk
4. Risk validates and forwards order over its single stream to Matching Engine
5. Matching engine processes, sends FILL messages back to Risk
6. Risk updates user positions, forwards fills to Gateway
7. Gateway forwards fills to user

**Stream semantics:**
- Long-lived (duration of process uptime)
- Bidirectional: Gateway → Risk → Matching (ORDER, CANCEL), reverse for FILL, ORDER_DONE
- Multiplexed by user_id and symbol (no per-user streams)
- Closed only on process shutdown or reconnect

**Transport options:**
- v1: gRPC over TCP/UDS
- No v2 planned (see FUTURE.md)

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
See PROTOCOL.md for message format definitions.

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
- gRPC (native clients)

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
- gRPC over UDS (Unix Domain Sockets)
- No TLS (process isolation via OS)
- Lowest latency (~50-100us for gRPC, reference UDS.md)

**Cross-machine (same private VLAN):**
- gRPC over TCP
- IPsec at the network layer (no per-message cost)

**Cross-machine (untrusted network):**
- Not supported in v1 (internal IPsec required)

## Performance Characteristics

### External → Gateway

**Latency:**
- Internet → TLS handshake: ~50-200ms (initial)
- JSON WebSocket message: ~1-10ms (after handshake)
- gRPC web message: ~1-5ms (lower serialization overhead)

**Bottleneck:**
- Network latency (internet → data center)
- TLS encryption/decryption (mitigated by connection reuse)

### Gateway → Risk → Matching Engine

**Latency (same machine, gRPC over UDS):**
- ~50-100us per message (reference UDS.md)
- Includes: gRPC frame, protobuf serialize, UDS write, kernel copy, protobuf deserialize

**Latency (cross-machine, gRPC over TCP):**
- ~100-300us per message (reference SMRB.md)
- Includes: gRPC frame, protobuf, TCP loopback, network switch

**Future optimization:**
- No v2 planned (see FUTURE.md)

## Deployment Topologies

### Single Machine (Development, Small Deployment)

```
┌─────────────────────────────────────┐
│         Machine 1                    │
│                                      │
│  Gateway ──UDS──→ Risk ──UDS──→ Matching BTC  │
│                      │                        │
│                      └─────────UDS──→ Matching ETH  │
└─────────────────────────────────────┘
```

**Benefits:**
- Lowest latency (UDS, no network)
- Simplest deployment (single binary, single config)

**Limits:**
- CPU cores (matching engines need dedicated cores)
- Memory (orderbook per symbol)

### Distributed (Production)

```
┌──────────────────┐       ┌──────────────────┐
│   Machine 1       │       │   Machine 2       │
│                   │       │                   │
│  Gateway1 ────────┼──TCP──┼──→ Risk ──TCP──→ Matching BTC   │
│  Gateway2 ────────┼──TCP──┼──→ Risk ──TCP──→ Matching ETH   │
└──────────────────┘       └──────────────────┘
       │                            │
       └──────────TCP───────────────┘
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
- Mitigation: UUIDv7 deduplication in matching engine (reference PROTOCOL.md)

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
- **PROTOCOL.md**: Message format, gRPC service definitions
- **WEBPROTO.md**: WebSocket overlay and compact wire protocol
