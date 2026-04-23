---
status: shipped
---

# Speedup Critique

Bottlenecks in current architecture, ordered by impact.

## End-to-End Estimate (v1)

```
User → Gateway:     1-100ms (internet)
Gateway → Risk:     ~10-50us (QUIC + WAL wire format)
Risk → ME:          ~10-50us (QUIC + WAL wire format)
ME match:           ~0.5us
ME → Risk:          ~10-50us (QUIC + WAL wire format)
Risk → Gateway:     ~10-50us (QUIC + WAL wire format)
Gateway → User:     1-100ms (internet)
─────────────────────────────────────────
Internal:           ~40-200us
With internet:      ~2-200ms
```

Internet dominates for web users. Internal QUIC overhead
matters for co-located traders (<100us network).

## v1: Ship With quinn QUIC + WAL Wire Format

quinn QUIC carrying raw #[repr(C)] records from day one.
Internet latency still dominates for most users. Focus on
what's cheap to fix now:

### monoio in Gateway (not Tokio)

Gateway is on the hot path. epoll = syscall per I/O op.
io_uring batches submissions, fewer syscalls, lower tail
latency. monoio proven in `../trader/monoio-client/`.
Drop-in swap inside the gateway tile.

### Deploy on NVMe Only

WAL fsync stalls ME on slow disk. NVMe: 0.5-5ms. Rotating
disk: 5-10ms. Deployment requirement, not code change.

### Single NUMA Socket

Avoid IPIs between Gateway and ME. Pin all hot-path
processes to same socket.

### Tune SPSC Ring Sizes

Per-consumer rings already designed. Market data ring
larger than risk ring. Slow mktdata consumer must not
stall matching.

## v1.5: Optimize Hot Path

### Batch Margin Recalculation

Risk recalculates margin for ALL exposed users on every
BBO update. 1000 users * 10 symbols = 10ms stall.
Fix: batch per 10ms, lazy eval, SIMD vectorization.

### Online Snapshots for Fast Recovery

Risk replays WAL from tips on startup. 1M records = ~2s
downtime. Fix: periodic snapshots, skip cold WAL, parallel
replay per symbol.

### Binary Protocol for Co-Located Clients

JSON parse: ~10us. Binary (raw structs or FlatBuffers):
<1us. Keep JSON for browsers, binary opt-in for bots.

### Configurable Postgres Batch Interval

10ms default. For low-latency deployments, 1ms. Trade-off
is throughput vs latency.

### Zone Recentering Off Hot Path

Orderbook recentering stalls ME for 10-100us on mid-price
drift. Move to background thread, prepare offline.

## v2: DPDK/AF_XDP for Userspace Networking

Already using quinn QUIC + WAL wire format. Next win is
userspace networking for gateway and market data.

Replace kernel networking with userspace NIC access. NIC
writes directly to ring buffers. No kernel, no syscalls.
Sub-microsecond gateway latency. Same tile architecture, swap
the I/O layer.

Only matters for co-located traders who need <10us.

## v3: RDMA for Same-Rack Communication

When processes run on separate machines in the same rack, QUIC
still hits kernel TCP/IP (~10-50us). RDMA bypasses everything:
NIC writes directly into remote machine's memory. No kernel,
no CPU on receive side, no serialization.

**RDMA + WAL wire format:**
- Same `#[repr(C, align(64))]` fixed records
- NIC DMA's the struct into a ring buffer on the remote
  machine. Remote CPU polls the ring -- same pattern as
  intra-process SPSC but across machines.
- Latency: ~1-2us (InfiniBand) or ~2-5us (RoCEv2)
- Compare: QUIC 10-50us, RDMA 1-5us

**Hardware options:**
- InfiniBand: ~1us, dedicated fabric, expensive switches
- RoCEv2: ~2-5us, RDMA over commodity Ethernet, cheaper
- Solarflare/OpenOnload: ~3-5us, kernel bypass TCP,
  drop-in socket replacement, needs Solarflare NICs
- CXL: ~200-400ns, actual shared memory over PCIe,
  bleeding edge hardware

**What changes from v2:**
- Swap QUIC transport for RDMA verbs (ibverbs)
- Ring buffer on remote machine replaces network socket
- Same WAL wire format, same records, same bytes
- Producer RDMA-writes into remote ring, consumer polls
- Fallback to QUIC for cross-datacenter links

## Capacity Estimate (Single Machine)

Target: c7i.metal-48xl (192 vCPUs / 96 physical cores,
384 GiB RAM, 100 Gbps network, EFA support).

```
Component           Cores   Capacity
─────────────────────────────────────
ME tiles            50      50 symbols (1 core each)
Risk shards         10      ~10K users (1K per shard)
Gateway (monoio)    4-8     10K WS connections
WAL Writer          2       2 streams
Marketdata          4       50 symbols fan-out
Postgres write-behind 2     batched flushes
DXS Replay          2       external consumers
Spare               ~20
─────────────────────────────────────
Total               ~96 cores
```

RAM: 50 orderbooks * 15MB = 750MB. 10K user positions =
trivial. RAM is not the constraint.

**10K users, 50 symbols on one machine.** Matches DEPLOY.md.

**Scaling past one machine (100K users):**
- 100 Risk shards = 100 cores (needs 2+ machines)
- Gateway shards by user_id hash (multiple machines)
- ME stays single-machine per symbol
- Inter-machine transport: QUIC (v2), RDMA/EFA (v3)

**AWS EFA:** ~5-10us latency, kernel bypass, SRD protocol.
Not true InfiniBand (~1us) but 10x better than QUIC.
Available on c7i.metal, c6in.metal, hpc6a instances.
For true ~1us RDMA: bare metal colo (Equinix etc).

## Priority Summary

```
v1 (ship now):
  - quinn QUIC + WAL wire format between processes
  - monoio in Gateway
  - NVMe, single NUMA socket
  - Tune SPSC ring sizes

v1.5 (optimize):
  - Batch margin recalculation
  - Online snapshots
  - Binary client protocol
  - Configurable Postgres batch

v2 (userspace networking):
  - DPDK/AF_XDP for gateway and market data

v3 (same-rack):
  - RDMA (InfiniBand or RoCEv2) + WAL wire format
  - NIC-to-NIC memory writes, ~1-5us
  - Fallback to QUIC for cross-datacenter
```
