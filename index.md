# RSX Knowledge Base

## Specs (v1)

### Architecture & Communication

- [NETWORK.md](specs/v1/NETWORK.md) - System topology: Gateway -> Risk -> Matching, scaling strategy, multiplexed gRPC streams
- [WEBPROTO.md](specs/v1/WEBPROTO.md) - WebSocket overlay: compact wire protocol for web clients
- [RPC.md](specs/v1/RPC.md) - Async RPC: UUIDv7 tracking, LIFO VecDeque optimization, backpressure, error handling
- [GRPC.md](specs/v1/GRPC.md) - gRPC proto: order states, message definitions, fill streaming, completion signals
- [MARKETDATA.md](specs/v1/MARKETDATA.md) - Market data gRPC: BBO + L2 snapshot/delta schema
- [CONSISTENCY.md](specs/v1/CONSISTENCY.md) - Event fan-out: SPSC delivery to risk/persistence/market data, ordering guarantees, failure handling
- [WAL.md](specs/v1/WAL.md) - Shared WAL architecture: bounded loss windows, backpressure, replica sync
- [ARCHIVE.md](specs/v1/ARCHIVE.md) - WAL offload + replay from flat files
- [DATABASE.md](specs/v1/DATABASE.md) - Async write-behind persistence architecture
- [METADATA.md](specs/v1/METADATA.md) - Scheduled symbol config: ticks, fees, limits, risk params

### Matching Engine

- [ORDERBOOK.md](specs/v1/ORDERBOOK.md) - Orderbook data structures, matching algorithm, tick/lot size curves, memory layout

### Risk Engine

- [RISK.md](specs/v1/RISK.md) - Pre-trade margin, position tracking, funding, persistence, replication

### Testing

- [TESTING.md](specs/v1/TESTING.md) - Testing strategy: unit/e2e/integration/smoke/perf targets, test data patterns, correctness invariants, CI/CD pipeline

### Guarantees & Recovery

- [GUARANTEES.md](GUARANTEES.md) - Formal system guarantees: consistency model, durability bounds, failure scenarios, data loss limits, recovery objectives
- [RECOVERY-RUNBOOK.md](RECOVERY-RUNBOOK.md) - Operational recovery procedures: detection, triage, step-by-step recovery for each failure scenario
- [CRASH-SCENARIOS.md](CRASH-SCENARIOS.md) - Detailed crash analysis: 15+ failure scenarios with preconditions, effects, recovery paths, verification
- [MONITORING.md](MONITORING.md) - Metrics, dashboards, alerts: track guarantees, detect violations, measure performance

## Specs (v2)

- [FUTURE.md](specs/v2/FUTURE.md) - Future optimizations: transport layer, protocol improvements, performance enhancements
- [ORDERBOOKv2.md](specs/v2/ORDERBOOKv2.md) - V2 orderbook: variable tick/lot sizes (price-dependent bands)

## Notes

- [SMRB.md](notes/SMRB.md) - Low-latency IPC: SPSC ring buffers, protocol comparison, SSL/TLS guidance, performance optimizations
- [UDS.md](notes/UDS.md) - UDS vs shared memory: latency/throughput comparison, hybrid patterns, Rust examples
- [ARENA.md](notes/ARENA.md) - Arena/bump allocators: fast allocation, bulk-free, graph-friendly lifetimes, Rust allocator landscape
- [HOTCOLD.md](notes/HOTCOLD.md) - Hot/cold field splitting: cache-friendly data layouts, SoA vs AoS, ECS connection
- [ALIGN.md](notes/ALIGN.md) - Why `repr(C, align(64))`: deterministic layout, cache line alignment, false sharing prevention

## Blog

- [Don't YOLO Structs Over The Wire](blog/dont-yolo-structs-over-the-wire.md) - The 9 risks of raw `#[repr(C)]` structs: alignment, endianness, versioning, torn reads, unsafe transmute, invalid enums, floats, DoS, framing
- [FlatBuffers Isn't Free](blog/flatbuffers-isnt-free.md) - Write-side overhead, wire size bloat, pointer chasing, ecosystem maturity, awkward mutation
- [Picking a Wire Format](blog/picking-a-wire-format.md) - Raw structs vs protobuf vs FlatBuffers vs Cap'n Proto: latency/safety/evolution trade-offs, hybrid strategy
