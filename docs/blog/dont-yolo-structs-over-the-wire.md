# Don't YOLO Structs Over The Wire

You're building a low-latency trading system. You need to pass market data
between processes. The hot path is microseconds-sensitive. Someone suggests:
"Just `#[repr(C)]` the struct and blast it over a socket. Zero serialization
overhead!"

This works. Until it doesn't.

Here are the 9 ways raw structs over the wire will bite you in production.

## 1. Alignment and Padding

```rust
#[repr(C)]
struct Trade {
    price: f64,      // 8 bytes, aligned to 8
    qty: u32,        // 4 bytes
    side: u8,        // 1 byte
    // invisible: 3 bytes of padding
}

// sizeof(Trade) == 16, not 13
```

Send this over the wire. The receiving process has a slightly different
compiler version, different target, or just different compilation flags. The
padding changes. Boom—misaligned reads, garbage data, silent corruption.

**What breaks:** Cross-platform communication, upgrades, debug vs release
builds.

## 2. Endianness

```rust
let price: u64 = 100_000;
// Little-endian: [0xa0, 0x86, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00]
// Big-endian:    [0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x86, 0xa0]
```

Your gateway runs on x86 (little-endian). Your FPGA or network appliance is
big-endian. You send `100_000`. It receives `2752512000`.

**What breaks:** Hardware diversity, FPGAs, network appliances, cross-arch
communication.

## 3. Versioning and Evolution

```rust
// Week 1
#[repr(C)]
struct Order {
    id: u64,
    price: f64,
    qty: u32,
}

// Week 2: add execution timestamp
#[repr(C)]
struct Order {
    id: u64,
    price: f64,
    qty: u32,
    exec_ts: u64,  // new field
}
```

Old receiver gets new struct. Reads `exec_ts` as garbage. Or worse: reads past
buffer end, segfault.

New receiver gets old struct. `exec_ts` is uninitialized memory. Or it reads
the next message's header as `exec_ts`.

**What breaks:** Rolling upgrades, A/B testing, multi-version deployments.

## 4. Torn Reads

```rust
#[repr(C)]
struct BookUpdate {
    symbol: u32,
    bid: f64,
    ask: f64,
    qty: u64,
}
```

Writer updates this in shared memory. Reader observes mid-write: old `bid`,
new `ask`. Now your spread is negative. Your trading engine goes haywire.

Even worse with sockets: partial writes. TCP gives you `send()` but no
atomicity. You write 24 bytes, it sends 16, then blocks. Reader gets torn
struct, interprets tail as next message header.

**What breaks:** Shared memory without atomics, TCP streams, high write rates.

## 5. Unsafe Transmute Hell

```rust
let bytes: [u8; 24] = recv_from_socket();
let trade: Trade = unsafe { std::mem::transmute(bytes) };
```

This is the actual operation under the hood. You're telling Rust: "Trust me,
these bytes are a valid `Trade`."

What if they're not? What if `side` is 42 instead of 0/1? What if `price` is
NaN? What if `qty` is `u32::MAX` and you're about to multiply it by price?

**What breaks:** Everything. Undefined behavior. Nasal demons. Production
chaos.

## 6. Invalid Enums

```rust
#[repr(u8)]
enum Side {
    Bid = 0,
    Ask = 1,
}

#[repr(C)]
struct Trade {
    side: Side,
    price: f64,
    qty: u32,
}
```

Receive bytes `[0x02, ...]`. Transmute to `Trade`. `trade.side` is now 2.
Rust's enum invariants are violated. Compiler assumed only 0 or 1. Optimized
match arms away. Now you're in undefined behavior land.

```rust
match trade.side {
    Side::Bid => do_bid(),
    Side::Ask => do_ask(),
    // compiler: "no other cases possible, no need for bounds check"
    // reality: trade.side == 2, processor executes whatever is at that memory
}
```

**What breaks:** Enum safety, match exhaustiveness, optimizer assumptions.

## 7. Floats Are Not Portable

```rust
let price: f32 = 123.45;
```

IEEE 754 is standard, right? Mostly. But:
- NaN payloads differ
- Signaling vs quiet NaN differs
- Denormal handling differs (flush-to-zero on some chips)
- `-0.0` vs `+0.0` comparison behavior

Send a NaN over the wire. Receiving CPU has different NaN handling. Your
careful validation logic breaks.

**What breaks:** NaN propagation, denormal numbers, cross-CPU consistency.

## 8. Denial of Service

```rust
#[repr(C)]
struct Message {
    len: u32,
    payload: [u8; 1024],
}
```

Attacker sends `len: 0xFFFFFFFF`. Your code allocates `u32::MAX` bytes. OOM
killer arrives.

Or: `len: 0`. You skip processing but still ack. Now sender can flood you with
zero-length messages, burning CPU on socket reads.

**What breaks:** Resource limits, adversarial inputs, production stability.

## 9. Framing and Delimiters

```rust
loop {
    let mut buf = [0u8; 32];
    socket.read_exact(&mut buf)?;
    let trade: Trade = unsafe { std::mem::transmute(buf) };
    process(trade);
}
```

Works fine until:
- Socket recv returns partial read (slow network, kernel buffers)
- Sender writes two messages back-to-back (coalesced TCP packets)
- You're now reading half of message N and half of message N+1

No length prefix, no delimiter, no framing. Your struct boundaries drift.
Silent corruption.

**What breaks:** TCP streams, buffering, burst writes, network variability.

## Mitigations

Don't give up on raw structs entirely. Use `zerocopy` or `bytemuck` to make
transmute safe:

```rust
use zerocopy::{FromBytes, AsBytes, Unaligned};

#[repr(C)]
#[derive(FromBytes, AsBytes, Unaligned)]
struct Trade {
    price: u64,  // fixed-point, not float
    qty: u32,
    side: u8,
    _pad: [u8; 3],  // explicit padding
}

let bytes: &[u8] = recv_from_socket();
let trade = Trade::read_from(bytes)?;  // validates alignment, size
```

This catches most of the footguns:
- Padding is explicit
- Alignment validated at runtime
- Size checked before transmute
- No floats, no enums

But you still have:
- Endianness (use `u64::from_le_bytes()`)
- Versioning (length prefix + version field)
- Torn reads (atomic writes or message boundaries)

Raw structs are fast. They're also sharp. Know the edges before you cut
yourself.
