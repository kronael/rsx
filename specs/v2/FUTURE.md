# Future Optimizations & Improvements

This document collects optimization ideas and protocol improvements that are
NOT part of v1 implementation. These are archival notes only. There is no v2
planned at this time.

## Transport Layer

### Raw Structs Over SMRB (Same Machine)

**Current (v1):** gRPC + protobuf over UDS (~50-100us)

**Archived (not planned):**
- Raw `#[repr(C)]` structs over SPSC ring buffer (reference SMRB.md)
- Latency: ~50-200ns (500x faster than gRPC)
- Removes: gRPC framing, protobuf serialization, kernel copy
- Requires: same machine, same binary version

**Trade-offs:**
- **Faster:** 50-200ns vs 50-100us (reference blog/picking-a-wire-format.md)
- **Simpler:** direct memory copy, no serialization
- **More fragile:** no schema evolution, version breaks everything
- **Same-machine only:** cannot use across processes on different machines

**Implementation:**
```rust
#[repr(C)]
#[derive(Copy, Clone, AsBytes, FromBytes)]
struct OrderMessage {
    order_id: [u8; 16],  // UUIDv7
    user_id: u32,
    symbol: u32,
    side: u8,
    _pad1: [u8; 3],
    price: i64,
    qty: i64,
    timestamp_ns: u64,
}

// Write
let msg = OrderMessage { ... };
smrb.write(bytemuck::bytes_of(&msg));

// Read (zero-copy)
let bytes = smrb.read();
let msg: &OrderMessage = bytemuck::from_bytes(bytes);
```

**Reference:**
- SMRB.md: SPSC ring buffer design
- blog/picking-a-wire-format.md: Raw structs vs protobuf trade-offs
- blog/dont-yolo-structs-over-the-wire.md: Safety considerations

### FlatBuffers for External API

**Current (v1):** gRPC + protobuf for all APIs

**Archived (not planned):**
- Use FlatBuffers for external user-facing API (WebSocket, REST)
- Keep gRPC + protobuf for internal Gateway ↔ Matching Engine
- Or use raw structs for internal (if same machine)

**Why FlatBuffers for external:**
- Schema evolution (add/remove fields without breaking clients)
- Zero-copy reads (client can parse without full deserialization)
- Better for broadcast (market data to thousands of clients)

**Why NOT for internal:**
- Write-side overhead (180-800ns, reference blog/flatbuffers-isnt-free.md)
- Wire size bloat (2-4x vs raw structs)
- Rust support is weak (verbose, awkward API)

**Hybrid strategy:**
```
External (WebSocket) ──FlatBuffers──→ Gateway
                                        │
Internal (UDS)       ──Raw structs───→ Matching Engine
```

**Reference:**
- blog/picking-a-wire-format.md: When to use FlatBuffers
- blog/flatbuffers-isnt-free.md: Write overhead, wire bloat

### TLS for Cross-Machine Communication

**Current (v1):** gRPC over TCP, no TLS (private VLAN, trusted network)

**Archived (not planned):**
- Add TLS 1.3 encryption for Gateway ↔ Matching Engine
- Performance cost: ~50-100us extra per message (reference SMRB.md)
- Alternative: IPsec at network layer (no per-message cost)

**When to use:**
- Cross-data-center communication (internet transit)
- Shared network (multi-tenant, untrusted environment)
- Compliance requirements (encrypt data in transit)

**When to skip:**
- Same machine (UDS, process isolation via OS)
- Private VLAN (physically isolated network)
- Performance-critical (TLS overhead too high)

**Reference:**
- SMRB.md section "SSL/TLS Considerations"
- UDS.md: UDS vs TCP latency comparison

### Userspace Networking (DPDK-Style)

**Current (v1):** Kernel networking (TCP, gRPC, standard socket API)

**Archived (not planned):**
- Userspace networking: bypass kernel, direct NIC access
- Technologies: DPDK, io_uring, AF_XDP
- Latency: ~1-5us (kernel bypass, zero-copy)

**Trade-offs:**
- **Much faster:** ~1-5us vs ~50-100us (kernel bypass)
- **Much harder:** custom network stack, NIC drivers, packet parsing
- **Less portable:** hardware-specific, NIC compatibility
- **Overkill for v1:** gRPC is fast enough (<100us)

**When to consider:**
- Sub-10us latency requirement (HFT, market making)
- Dedicated hardware (FPGA, SmartNIC)
- Extreme throughput (millions of messages/sec)

**Reference:**
- firedancer (Solana validator, DPDK-based networking)
- Solarflare Onload (kernel bypass for ultra-low latency)

## Protocol Optimizations

### Replace Protobuf with Raw Structs + Zerocopy

**Current (v1):** Protobuf serialization (~50-200ns encode, ~20-100ns decode)

**Archived (not planned):**
- Raw `#[repr(C)]` structs with `zerocopy` crate
- Zero serialization (direct memory copy)
- Requires: same binary version, same endianness

**Implementation:**
```rust
use zerocopy::{AsBytes, FromBytes, Unaligned};

#[repr(C)]
#[derive(Copy, Clone, AsBytes, FromBytes, Unaligned)]
struct OrderMessage {
    msg_type: u8,        // Discriminant: 0=NewOrder, 1=Cancel, etc.
    _pad: [u8; 7],
    order_id: [u8; 16],
    user_id: u32,
    symbol: u32,
    side: u8,
    _pad2: [u8; 3],
    price: i64,
    qty: i64,
    timestamp_ns: u64,
}
// Total: 64 bytes (one cache line)

// Write
let msg = OrderMessage { ... };
smrb.write(msg.as_bytes());

// Read
let bytes = smrb.read();
let msg = OrderMessage::read_from(bytes).unwrap();
```

**Benefits:**
- Faster: ~5-10ns vs ~50-200ns (protobuf)
- Smaller: 64 bytes vs ~80-100 bytes (protobuf varint overhead)
- Cache-friendly: fixed size, aligned, no pointer chasing

**Costs:**
- No schema evolution (version change breaks everything)
- Platform-dependent (endianness, alignment)
- Must coordinate binary updates (all components same version)

### Batch Fills (Vec<Fill> Instead of Streaming)

**Current (v1):** Stream individual FILL messages
```
Matching Engine ──FILL(qty=10)──→ Gateway
                ──FILL(qty=20)──→ Gateway
                ──FILL(qty=30)──→ Gateway
                ──ORDER_DONE────→ Gateway
```

**Archived (not planned):** Batch fills in single message
```
Matching Engine ──ORDER_RESULT(fills=[10,20,30], status=FILLED)──→ Gateway
```

**Benefits:**
- Fewer messages (1 vs 4)
- Lower overhead (less framing, less syscalls)
- Easier parsing (one message, all fills)

**Costs:**
- Larger messages (all fills in one)
- No incremental updates (user waits for all fills)
- More complex parsing (variable-length array)

**When to use:**
- High-throughput symbols (BTC-PERP, ETH-PERP)
- Many fills per order (market orders sweeping orderbook)
- Network bandwidth limited (fewer packets)

**When to skip:**
- Low-latency requirement (stream fills immediately)
- Few fills per order (limit orders, 1-2 fills)
- User wants incremental updates (show fills as they happen)

### Pre-Allocated Message Pools (Zero Allocation on Hot Path)

**Current (v1):** Allocate protobuf messages on heap (Vec, String)

**Future (v2):**
- Pre-allocate message pool (e.g., 10,000 message slots)
- Reuse messages (no malloc/free during matching)
- Zero allocation on hot path

**Implementation:**
```rust
struct MessagePool<T> {
    pool: Vec<T>,
    free_list: Vec<usize>,
}

impl<T: Default> MessagePool<T> {
    fn alloc(&mut self) -> &mut T {
        let idx = self.free_list.pop().unwrap();
        &mut self.pool[idx]
    }

    fn free(&mut self, idx: usize) {
        self.free_list.push(idx);
    }
}

// Usage
let msg = msg_pool.alloc();
msg.order_id = order_id;
msg.price = price;
smrb.write(msg);
msg_pool.free(msg_idx);
```

**Benefits:**
- Zero allocation (no malloc/free during matching)
- Cache-friendly (pool is contiguous memory)
- Faster (avoid allocator overhead)

**Costs:**
- Fixed pool size (must estimate max concurrent messages)
- More complex (manual pool management)
- Memory overhead (pre-allocate all slots, even if unused)

**When to use:**
- Ultra-low latency (sub-microsecond matching)
- Allocation is bottleneck (profiler shows malloc in hot path)
- Dedicated hardware (enough memory for large pool)

## Other Future Improvements

### Modify Order (Amend Price/Qty Without Cancel/Replace)

**Current (v1):** Modify = cancel + new order (loses time priority)

**Future:**
- MODIFY message (change price/qty in-place)
- Preserves time priority if qty decreases
- Loses time priority if price changes or qty increases

**Use case:**
- Market makers adjusting quotes frequently
- Avoid cancel/replace overhead (two messages, two roundtrips)

### Stop-Loss / Take-Profit Orders (Conditional Orders)

**Current (v1):** GTC limit orders only

**Future:**
- Stop-loss: trigger market order when price hits threshold
- Take-profit: trigger limit order when price hits threshold
- Requires: Matching Engine tracks trigger conditions

**Use case:**
- Retail traders want automated exit strategies
- Reduce Gateway load (no need for client-side monitoring)

### Batch Order Submission (Submit Multiple Orders at Once)

**Current (v1):** One NewOrder message per order

**Future:**
- BatchNewOrder: submit 10-100 orders in one message
- Reduces network overhead (one message, one roundtrip)

**Use case:**
- Market makers submitting ladder of orders (10 bids, 10 asks)
- Initial margin calculation on batch (more efficient)

### Order Book Snapshots (For Market Data Clients)

**Current (v1):** No market data dissemination in protocol

**Future:**
- OrderBookSnapshot message (top-N levels, bid/ask)
- Incremental updates (OrderBookDelta)
- Broadcast to subscribers

**Use case:**
- Market data feeds for clients
- Trading bots need orderbook depth

### Cross-Symbol Margin (Portfolio Margining)

**Current (v1):** Risk checks per symbol (isolated margin)

**Future:**
- Cross-symbol margin: offset BTC long with ETH short
- Requires: Gateway tracks portfolio-level risk
- More capital efficient (lower margin requirements)

**Use case:**
- Sophisticated traders hedging across symbols
- Market makers providing liquidity on correlated pairs

---

**Note:** This file is a collection point for future ideas. None of these are
implemented in v1. They are noted for reference, prioritization, and planning.

**When adding items:**
- Describe current state (v1) vs future state
- Explain benefits and costs (trade-offs)
- Reference existing docs (ORDERBOOK.md, RPC.md, etc.)
- Include use cases (when to use, when to skip)
