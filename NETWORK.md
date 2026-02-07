# System Network Topology

## Overview

RSX uses a two-tier architecture separating user-scoped concerns (gateway,
auth, risk) from symbol-scoped execution (matching engine). Both tiers run as
monolithic processes — no distributed consensus, no Raft, no cross-process
coordination within a tier.

```
External                Internal               Execution
─────────────────────   ────────────────────   ──────────────────
Web Users (JSON/WS)
                  ↘
Native Clients     ──→  Gateway/Risk Engine  ──→  Matching Engine
(gRPC web)             (monolithic process)      (one per symbol)
                  ↗                               (monolithic)
Mobile Apps (gRPC)
```

## Why This Topology

**Gateway + Risk Engine merged:**
- Both user-scoped (operate on user positions, balances, sessions)
- Both monolithic (no sharding within a component)
- Merging eliminates one network hop (auth → risk → matching becomes auth+risk → matching)
- Shared context: user balances, positions, margin calculations
- Simpler: fewer moving parts, fewer failure modes

**Matching Engine separate:**
- Symbol-scoped (one process per symbol or symbol group)
- Single-threaded per symbol (no locks, cache-friendly, reference ORDERBOOK.md)
- Stateless regarding users (just order IDs, no position tracking)
- Scales horizontally: add symbols by adding processes
- Clean isolation: BTC-PERP and ETH-PERP cannot interfere

## Component Architecture

### Gateway/Risk Engine (Merged)

**Responsibilities:**
- WebSocket JSON API for web clients (design only, not v1 implementation)
- gRPC web passthrough for native clients (v1 focus)
- User authentication and session management
- Rate limiting per user/IP
- Position tracking (long/short qty per symbol)
- Margin calculation (initial margin, maintenance margin)
- Risk checks BEFORE sending orders to matching engine
- Fill ingestion and position update after matching

**Architecture:**
- Monolithic process (can handle thousands of users, but single process)
- Async runtime (Tokio) for concurrent user sessions
- One task per user WebSocket/gRPC stream
- Horizontal scaling: shard by user ID hash (load balancer routes by user_id)
- No cross-instance coordination (each instance owns its users)

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
- Event emission to Gateway/Risk via bidirectional gRPC stream

**Scaling:**
- Horizontal by symbol: one process per symbol or symbol group
- High-volume symbols get dedicated processes (BTC-PERP, ETH-PERP)
- Low-volume symbols can be grouped (all memecoins in one process)
- No cross-symbol coordination in v1

## Scaling Strategy

### Gateway/Risk: User Sharding

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
- Each gateway must connect to ALL active matching engines
- User can trade any symbol, so all gateways talk to all matchers
- Gateway-to-matcher streams are long-lived (one per active user per symbol)

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

**WebSocket JSON API (design only, not v1):**
- TLS encrypted
- JSON message framing
- Authentication via JWT in headers
- Session management via WebSocket connection ID
- Not implemented in v1 (future work)

**gRPC Web (v1 implementation):**
- TLS encrypted
- gRPC streaming (bidirectional)
- Authentication via gRPC metadata (JWT)
- Native clients (desktop, mobile, trading bots)
- Protocol defined in PROTOCOL.md

### Internal: Gateway ↔ Matching Engine

**Transport:**
- gRPC bidirectional streaming (v1)
- One stream per user session per symbol
- Example: User trading BTC-PERP and ETH-PERP = 2 streams

**Connection lifecycle:**
1. User opens WebSocket/gRPC connection to Gateway
2. User sends order for BTC-PERP
3. Gateway opens bidirectional stream to BTC-PERP matching engine (if not already open)
4. Gateway validates risk, sends ORDER message to matching engine
5. Matching engine processes, sends FILL messages back
6. Gateway updates user positions, forwards fills to user

**Stream semantics:**
- Long-lived (duration of user session)
- Bidirectional: Gateway → Matching (ORDER, CANCEL), Matching → Gateway (FILL, ORDER_DONE)
- One stream per (user_id, symbol) pair
- Closed when user disconnects or stops trading that symbol

**Transport options (evolution path):**
```
v1: gRPC over TCP/UDS
  ↓ (replace serialization)
v2: Raw structs over SMRB (same machine)
  ↓ (add TLS for cross-machine)
v3: Raw structs over TCP/TLS (cross-machine)
```

See SMRB.md, UDS.md, and blog/picking-a-wire-format.md for trade-offs.

## Data Flow

### Order Submission Flow

```
User ──ORDER──→ Gateway/Risk
                   │
                   ├─ Authenticate
                   ├─ Rate limit check
                   ├─ Margin check (risk)
                   │
                   ├─ Assign UUIDv7 order ID
                   ├─ Add to pending VecDeque
                   │
                   └──ORDER──→ Matching Engine
                                  │
                                  ├─ Validate tick/lot size
                                  ├─ Match against orderbook
                                  ├─ Generate FILL events
                                  │
                                  ├──FILL──→ Gateway (0+ times)
                                  └──ORDER_DONE/FAILED──→ Gateway
                                     │
                                     ├─ Pop from pending VecDeque
                                     ├─ Update user positions
                                     ├─ Recalculate margin
                                     │
                                     └──FILL/DONE──→ User
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
Gateway receives FILL
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
- JSON over WebSocket (web clients, design only)
- gRPC web (native clients, v1 focus)

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
- gRPC over TCP (no TLS in v1)
- Private network, trusted environment
- Optional: IPsec at network layer (no per-message cost)

**Cross-machine (untrusted network):**
- gRPC over TCP + TLS
- Performance cost: ~50-100us extra per message (reference SMRB.md)

## Performance Characteristics

### External → Gateway

**Latency:**
- Internet → TLS handshake: ~50-200ms (initial)
- JSON WebSocket message: ~1-10ms (after handshake)
- gRPC web message: ~1-5ms (lower serialization overhead)

**Bottleneck:**
- Network latency (internet → data center)
- TLS encryption/decryption (mitigated by connection reuse)

### Gateway → Matching Engine

**Latency (same machine, gRPC over UDS):**
- ~50-100us per message (reference UDS.md)
- Includes: gRPC frame, protobuf serialize, UDS write, kernel copy, protobuf deserialize

**Latency (cross-machine, gRPC over TCP):**
- ~100-300us per message (reference SMRB.md)
- Includes: gRPC frame, protobuf, TCP loopback, network switch

**Future optimization (raw structs over SMRB, same machine):**
- ~50-200ns per message (reference SMRB.md, blog/picking-a-wire-format.md)
- Removes: gRPC overhead, protobuf serialization, kernel copy
- Adds: manual framing, no schema evolution, same-machine only

## Deployment Topologies

### Single Machine (Development, Small Deployment)

```
┌─────────────────────────────────────┐
│         Machine 1                    │
│                                      │
│  Gateway/Risk ──UDS──→ Matching BTC  │
│       │                              │
│       └─────────UDS──→ Matching ETH  │
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
│  Gateway1 ────────┼──TCP──┼──→ Matching BTC   │
│  Gateway2 ────────┼──TCP──┼──→ Matching ETH   │
└──────────────────┘       └──────────────────┘
       │                            │
       └──────────TCP───────────────┘
          (all gateways talk to all matchers)
```

**Benefits:**
- Horizontal scaling (add gateway/matcher machines independently)
- Isolation (matching engine failures don't kill gateways)
- Dedicated hardware per component (gateways need network I/O, matchers need CPU/cache)

**Complexity:**
- Network configuration (private VLAN, routing)
- Service discovery (how gateways find matching engines)
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

### Matching Engine Failure

**Impact:**
- Symbol becomes untradable (e.g., BTC-PERP down)
- All gateways lose streams to that matching engine
- Other symbols unaffected (ETH-PERP still works)

**Recovery:**
- Restart matching engine process
- Rebuild orderbook from persistence layer (order log)
- Gateways reconnect streams
- Users can submit new orders

**Orderbook persistence:**
- Append-only order log (every order, cancel, fill)
- Replay log on startup (rebuild orderbook state)
- Snapshot periodically (reduce replay time)

### Network Partition

**Gateway ↔ Matching Engine partition:**
- Gateway cannot send orders to matching engine
- Gateway queues orders OR returns error to user (config-dependent)
- Risk: user sees order rejected, but it was already sent (duplicate after reconnect)
- Mitigation: UUIDv7 deduplication in matching engine (reference PROTOCOL.md)

## Cross-References

- **ORDERBOOK.md**: Matching engine internals, orderbook data structure
- **SMRB.md**: Low-latency IPC options, SPSC ring buffer design
- **UDS.md**: UDS vs shared memory comparison, latency numbers
- **RPC.md**: Async request handling, pending order tracking
- **PROTOCOL.md**: Message format, gRPC service definitions
- **blog/picking-a-wire-format.md**: Why gRPC now, raw structs later
