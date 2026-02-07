# RSX Knowledge Base

## Architecture & Communication

- [NETWORK.md](NETWORK.md) - System topology: Gateway/Risk merge, scaling strategy, bidirectional gRPC streams
- [RPC.md](RPC.md) - Async RPC: UUIDv7 tracking, LIFO VecDeque optimization, backpressure, error handling
- [PROTOCOL.md](PROTOCOL.md) - Wire protocol: order states, gRPC message definitions, fill streaming, completion signals
- [SMRB.md](SMRB.md) - Low-latency IPC: SPSC ring buffers, protocol comparison, SSL/TLS guidance, performance optimizations
- [UDS.md](UDS.md) - UDS vs shared memory: latency/throughput comparison, hybrid patterns, Rust examples

## Matching Engine

- [ORDERBOOK.md](ORDERBOOK.md) - Orderbook data structures, matching algorithm, tick/lot size curves, memory layout

## Memory & Allocation

- [ARENA.md](ARENA.md) - Arena/bump allocators: fast allocation, bulk-free, graph-friendly lifetimes, Rust allocator landscape
- [HOTCOLD.md](HOTCOLD.md) - Hot/cold field splitting: cache-friendly data layouts, SoA vs AoS, ECS connection
- [ALIGN.md](ALIGN.md) - Why `repr(C, align(64))`: deterministic layout, cache line alignment, false sharing prevention

## Blog

- [Don't YOLO Structs Over The Wire](blog/dont-yolo-structs-over-the-wire.md) - The 9 risks of raw `#[repr(C)]` structs: alignment, endianness, versioning, torn reads, unsafe transmute, invalid enums, floats, DoS, framing
- [FlatBuffers Isn't Free](blog/flatbuffers-isnt-free.md) - Write-side overhead, wire size bloat, pointer chasing, ecosystem maturity, awkward mutation—when zero-copy reads cost more than you think
- [Picking a Wire Format](blog/picking-a-wire-format.md) - Raw structs vs protobuf vs FlatBuffers vs Cap'n Proto: latency/safety/evolution trade-offs, hybrid strategy, practical matching engine recommendations

## Future / V2

- [FUTURE.md](FUTURE.md) - Future optimizations: transport layer, protocol improvements, performance enhancements
- [ORDERBOOKv2.md](ORDERBOOKv2.md) - V2 orderbook: variable tick/lot sizes (price-dependent bands)
