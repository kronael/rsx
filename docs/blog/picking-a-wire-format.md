# Picking a Wire Format for Low-Latency Systems

You're building a matching engine. You need to move market data between
processes. Gateway → normalizer → strategy → execution engine. Maybe shared
memory, maybe sockets, definitely performance-critical.

What wire format?

## The Contenders

### Raw Structs (with `zerocopy`/`bytemuck`)

```rust
#[repr(C)]
#[derive(Copy, Clone, AsBytes, FromBytes)]
struct Trade {
    symbol: u32,
    price: u64,  // fixed-point
    qty: u32,
    side: u8,
    _pad: [u8; 3],
}

// Write
smrb.write(bytemuck::bytes_of(&trade));

// Read
let trade: &Trade = bytemuck::from_bytes(smrb.read());
```

**Pros:**
- Fastest possible (2-8ns ser/deser)
- Smallest wire size (no metadata)
- Zero allocations
- Cache-friendly (dense, inline)

**Cons:**
- No schema evolution (version breaks everything)
- No validation (garbage in = UB)
- Platform-dependent (endianness, alignment)
- No cross-language support

**Use when:** Internal IPC, same-version processes, controlled environment,
microsecond-sensitive hot path.

### Protocol Buffers (protobuf)

```proto
message Trade {
    uint32 symbol = 1;
    double price = 2;
    float qty = 3;
    Side side = 4;
}
```

**Pros:**
- Schema evolution (add/remove fields safely)
- Cross-language (20+ languages)
- Validation (type checking, required fields)
- Mature ecosystem

**Cons:**
- Slow (100-500ns serialize, 50-200ns deserialize)
- Wire size overhead (varint encoding, field tags)
- Allocates (temp buffers, strings)
- Poor cache locality (pointer chasing)

**Use when:** External APIs, multi-language systems, long-term data storage,
millisecond-scale latency budget.

### FlatBuffers

```fbs
table Trade {
    symbol: uint32;
    price: double;
    qty: float;
    side: Side;
}
```

**Pros:**
- Zero-copy reads (no deserialization)
- Schema evolution (vtables for compatibility)
- Forward/backward compatibility
- Validation (bounds checking)

**Cons:**
- Slow writes (180-800ns, builder overhead)
- Wire size bloat (2-4x vs raw, vtables)
- Pointer chasing (cache misses on nested data)
- Immutable (updates = rebuild)
- Rust support is weak

**Use when:** Read-heavy workloads, broadcast to many consumers, schema
evolution required, 10-100us latency budget.

### Cap'n Proto

```capnp
struct Trade {
    symbol @0 :UInt32;
    price @1 :Float64;
    qty @2 :Float32;
    side @3 :Side;
}
```

**Pros:**
- Zero-copy everything (read and write)
- No encode step (direct struct building)
- Faster than FlatBuffers (50-150ns)
- Better Rust support (capnp crate)

**Cons:**
- Wire size overhead (8-byte alignment everywhere)
- Less mature ecosystem
- Fewer language bindings
- Still pointer chasing for nested data

**Use when:** Like FlatBuffers, but Rust-first and willing to accept smaller
ecosystem.

## Latency vs Safety vs Evolution

```
                Raw Structs   Cap'n Proto   FlatBuffers   Protobuf
Latency         ★★★★★        ★★★★☆         ★★★☆☆         ★★☆☆☆
Safety          ★☆☆☆☆        ★★★★☆         ★★★★★         ★★★★★
Evolution       ☆☆☆☆☆        ★★★★☆         ★★★★★         ★★★★★
Wire Size       ★★★★★        ★★★☆☆         ★★☆☆☆         ★★★☆☆
Ergonomics      ★★★★★        ★★★☆☆         ★★☆☆☆         ★★★★☆
```

There's no free lunch. You're optimizing for latency, safety, or evolution.
Pick two.

## The Hybrid Strategy

Don't pick one format for everything. Use the right tool for each boundary.

### Architecture

```
Exchange (WebSocket)
  ↓ FlatBuffers (untrusted, versioned)
Gateway
  ↓ Raw structs (trusted, fast)
Normalizer
  ↓ Raw structs (SMRB)
Strategy
  ↓ Raw structs (SMRB)
Matching Engine
  ↓ Raw structs (SMRB)
Execution
  ↓ FlatBuffers (external API)
Exchange (WebSocket)
```

### External Boundaries: FlatBuffers

Why:
- Schema evolution (exchange updates API, you don't redeploy)
- Validation (untrusted input, malformed messages)
- Cross-language (exchange uses C++, you use Rust)

```rust
// Gateway receives from exchange
let fb_trade = flatbuffers::root::<Trade>(websocket_bytes)?;

// Validate and normalize
let trade = InternalTrade {
    symbol: Symbol::from_id(fb_trade.symbol()),
    price: Price::from_f64(fb_trade.price()),
    qty: Qty::from_f32(fb_trade.qty()),
    side: Side::try_from(fb_trade.side())?,
    exch_ts: fb_trade.timestamp(),
    recv_ts: now(),
};
```

### Internal Boundaries: Raw Structs

Why:
- Latency (normalizer → strategy → engine is <10us end-to-end)
- Throughput (100k msgs/sec, no GC pressure)
- Controlled (same version, same deployment, same endianness)

```rust
// Normalizer writes to SMRB
let bytes = bytemuck::bytes_of(&trade);
normalizer_to_strategy.write(bytes);

// Strategy reads from SMRB
let trade: &InternalTrade = bytemuck::from_bytes(
    strategy_to_engine.read()
);
```

### Boundary Conversion Pattern

The gateway is your adapter:

```rust
struct Gateway {
    websocket: WebSocket,
    smrb: SmrbWriter<InternalTrade>,
}

impl Gateway {
    fn on_message(&mut self, bytes: &[u8]) -> Result<()> {
        // Deserialize FlatBuffers
        let fb_trade = flatbuffers::root::<ExternalTrade>(bytes)?;

        // Validate (untrusted input)
        if fb_trade.price() <= 0.0 {
            return Err(Error::InvalidPrice);
        }

        // Normalize to internal representation
        let trade = InternalTrade {
            symbol: self.symbol_map.get(fb_trade.symbol())?,
            price: Price::from_f64(fb_trade.price()),
            qty: Qty::from_f32(fb_trade.qty()),
            side: Side::try_from(fb_trade.side())?,
            exch_ts: fb_trade.timestamp(),
            recv_ts: now(),
        };

        // Write raw struct to SMRB (hot path)
        self.smrb.write(bytemuck::bytes_of(&trade))?;
        Ok(())
    }
}
```

Now:
- External API is safe (validation, evolution)
- Internal path is fast (no ser/deser, no allocations)
- Decoupled (exchange schema changes don't affect matching engine)

## Practical Recommendation

### For a Matching Engine

1. **External APIs** (exchange, clients, monitoring):
   - Use FlatBuffers if evolution matters
   - Use Protobuf if cross-language matters
   - Use msgpack/JSON if latency doesn't matter

2. **Internal IPC** (gateway → normalizer → strategy → engine):
   - Use raw structs over SMRB
   - Add `zerocopy`/`bytemuck` for safety
   - Version the struct (rolling upgrades = rebuild all)

3. **Persistence** (order log, trade history):
   - Use FlatBuffers (schema evolution for replays)
   - Or use raw structs + version prefix (fast writes)

### For a Market Data System

1. **Ingest** (exchange → gateway):
   - FlatBuffers (exchange controls schema)

2. **Normalization** (gateway → normalizer):
   - Raw structs (controlled environment)

3. **Broadcast** (normalizer → N strategies):
   - Raw structs over shared memory
   - Each strategy gets zero-copy view
   - No ser/deser overhead

4. **Recording** (tick database):
   - FlatBuffers (schema evolves over years)

## Example: SMRB with Raw Structs

```rust
use zerocopy::{AsBytes, FromBytes, Unaligned};

#[repr(C)]
#[derive(Copy, Clone, AsBytes, FromBytes, Unaligned)]
struct BookUpdate {
    symbol: u32,
    bid_price: u64,  // fixed-point (*1e8)
    bid_qty: u32,
    ask_price: u64,
    ask_qty: u32,
    timestamp: u64,
}

// Producer
let update = BookUpdate {
    symbol: BTCUSD,
    bid_price: 50_000_00000000,
    bid_qty: 100,
    ask_price: 50_001_00000000,
    ask_qty: 150,
    timestamp: now(),
};
smrb.write(update.as_bytes());

// Consumer (zero-copy)
let bytes = smrb.read();
let update = BookUpdate::read_from(bytes).unwrap();

// Total latency: ~100ns (SMRB write + read + cache miss)
```

Compare to FlatBuffers: ~1000ns (build + write + read + vtable deref).

## When NOT to Use Raw Structs

- Untrusted input (internet-facing APIs)
- Cross-language systems (unless you're FFI-only)
- Long-term storage (can't read old data after version change)
- Distributed systems (endianness, alignment hell)

## Conclusion

Wire format is not a global choice. It's a per-boundary choice.

- **External = FlatBuffers/Protobuf** (safety, evolution)
- **Internal = Raw structs** (latency, throughput)
- **Boundary = Gateway** (convert once, run fast)

The matching engine's hot path should never touch a schema-evolved format.
That's what the gateway is for.

Build boundaries. Optimize interiors. Ship fast code.
