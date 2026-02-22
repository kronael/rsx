# Architecture-Preserving Latency Upgrades

We have a phased latency roadmap. Every phase changes exactly one
thing: the I/O layer inside the gateway tile. The SPSC rings, the
matching engine, and the risk engine are unchanged across all phases.

## Where the Latency Actually Is

Internal end-to-end: 40–200µs. That sounds fast. It's not the
interesting number.

```
User → Gateway:    1–100ms (internet)
Gateway → Risk:    10–50µs (QUIC + WAL wire format)
Risk → ME:         10–50µs
ME match:          ~0.5µs
ME → Risk:         10–50µs
Risk → Gateway:    10–50µs
Gateway → User:    1–100ms (internet)
```

For most users, network dominates. The internal 40–200µs is invisible
behind a 20ms internet RTT. Latency optimization inside the exchange
only matters for the cohort running within 1ms of the gateway — HFT
shops in the same datacenter.

But before you touch the I/O layer, measure the actual bottlenecks.
We found three, none of which are networking.

## Hidden Bottleneck 1: Margin Recalc Stalls

Every BBO update triggers a margin recalculation for every user with
an open position in that symbol. At 1,000 users across 10 symbols,
a single BBO event can trigger 10,000 recalculations inline on the
risk tile.

At ~1µs per recalc, that's 10ms of stall. During that 10ms, the risk
tile is not processing fills. The gateway queues are filling. Orders
are waiting.

The fix: batch recalc into a 10ms window, or move to lazy evaluation
triggered on query rather than on BBO update. SIMD vectorization of
the margin formula is a further improvement but secondary to the
batching.

This is not a networking problem. It's a scheduling problem.

## Hidden Bottleneck 2: Cold-Start Downtime

Risk replays from the ME WAL tip on startup. At 1 million WAL records,
replay takes approximately 2 seconds before the shard accepts live
orders.

Two seconds is a long time for a failover. The fix is periodic
snapshots: serialize shard state to disk every 10 minutes, start from
the snapshot on restart, replay only the delta. The snapshot code
exists in rsx-matching. The scheduler that triggers it does not.

Parallel replay per symbol is a further improvement: 50 symbols can
replay simultaneously, one goroutine each, instead of sequentially.

## Hidden Bottleneck 3: Zone Recentering on the Hot Path

The orderbook uses a price zone with a center point. When mid-price
drifts far enough from the center, the book recenters — a 10–100µs
operation that runs inline during matching.

During recentering, matching stalls. Orders that arrive during the
stall queue behind it. For a symbol with active mid-price drift, this
can fire multiple times per second.

Fix: prepare the new zone layout on a background thread. Swap it in
atomically. Matching continues on the old layout until the swap, then
picks up the new layout without stall.

## WAL fsync Numbers

Flush cadence matters more than hardware for p99.

NVMe: 0.5–5ms per fsync. Spinning disk: 5–10ms. The gap is a 10×
difference in worst-case WAL latency, which flows directly into p99
matching latency when backpressure triggers.

At the default 10ms flush cadence, you're paying 10ms of matching
stall every 10ms on slow disk. NVMe with the same cadence pays 5ms
average. The deployment requirement is NVMe-only.

## The Phased Roadmap

The tile architecture makes this clean. Each phase swaps the I/O
layer inside the gateway tile. Everything else is unchanged.

**Phase 1 (current): monoio/io_uring, QUIC**

Gateway uses monoio instead of tokio. io_uring batches syscalls,
reduces tail latency vs epoll. QUIC carries the WAL wire format
between processes. Per-hop latency: 10–50µs.

The gateway tile is a pinned thread with one SPSC downqueue (orders
in) and one SPSC upqueue (fills out). The I/O multiplexing inside the
tile is the only thing that changes in subsequent phases.

**Phase 2: DPDK or AF_XDP**

Replace kernel networking with userspace NIC access. NIC writes
directly into ring buffers. No kernel, no syscalls per packet.
Sub-microsecond gateway. Same tile architecture, same SPSC rings.

Only matters for the co-located cohort that needs <10µs.

**Phase 3: RDMA (InfiniBand or RoCEv2)**

When gateway and ME run on separate machines in the same rack, QUIC
still traverses the kernel network stack (~10–50µs). RDMA bypasses
everything: NIC DMA's the WAL struct into a ring buffer on the remote
machine. The remote CPU polls the ring. Same pattern as intra-process
SPSC, across machines.

- InfiniBand: ~1µs, dedicated fabric
- RoCEv2: ~2–5µs, RDMA over commodity Ethernet

**Phase 4: AWS EFA (cloud)**

Not InfiniBand. ~5–10µs. Kernel bypass, SRD protocol. Available on
c7i.metal-48xl (96 physical cores, 100 Gbps, EFA support). For the
cloud-deployed case, 10× better than QUIC with no colo requirement.

## Machine Ceiling

c7i.metal-48xl supports the full single-machine deployment:

```
ME tiles (1 core each):     50 symbols
Risk shards (1K users each): 10 shards = 10K users
Gateway (monoio):           4–8 cores, 10K WS connections
WAL writer, Marketdata:     6 cores
Spare:                      ~20 cores
```

RAM is not the constraint. 50 orderbooks at 15MB each is 750MB.
10K user positions is trivial.

Beyond one machine (100K users), Risk shards scale to multiple
machines. ME stays single-machine per symbol. Inter-machine transport
follows the phase roadmap above.

## The Rule

Don't optimize the I/O layer until you've measured the margin recalc
stall. In this class of system, the "latency" complaint usually traces
back to an inline 10ms batch computation, not to network overhead.

Measure. Fix the recalc. Fix the snapshot startup. Move zone
recentering off the hot path. Then look at the I/O layer.

## Industry Benchmark: Jump Trading vs RSX

From PJ Waskiewicz (Jump Trading), Netdev 0x16 (LWN #914992).
Jump's finding: **jitter matters more than average latency** —
unpredictable network jitter costs money; predictable 50µs beats
unpredictable 20µs.

| System | Latency | Scope | Notes |
|--------|---------|-------|-------|
| Solarflare (hardware) | ~30ns | end-to-end tick-to-trade | FPGA/ASIC, kernel bypass NIC |
| Jump (optimized Linux) | ~42µs | network round-trip | CPU pinning + IRQ affinity |
| Jump (baseline Linux) | ~52µs | network round-trip | standard kernel, no tuning |
| RSX v1 (QUIC) | ~10–50µs | IPC only (Gateway ↔ ME) | io_uring, kernel networking |
| RSX v2 (SMRB) | ~50–200ns | IPC only, same machine | shared memory, no syscall |
| RSX v3 (DPDK) | ~1–5µs | network + IPC (est.) | userspace networking |

RSX v1 internal IPC is already competitive with Jump's optimized
network stack. SMRB (phase 2) targets same-machine deployments.
Reaching Solarflare-class (<100ns) requires FPGA — not a CPU goal.

## OS Tuning for Predictable Latency

Jump's techniques (validated for production):

**Boot parameters:**
```
isolcpus=<ME cores>   — remove cores from kernel scheduler
nohz_full=<ME cores>  — eliminate timer ticks on those cores
rcu_nocbs=<ME cores>  — offload RCU callbacks off isolated cores
```

**IRQ affinity** — pin NIC interrupts to non-matching cores:
```
echo <cpu_mask> > /proc/irq/<nic_irq>/smp_affinity
```

**Runtime:**
- `numactl --cpunodebind=0 --membind=0` — Gateway and ME on same
  NUMA socket to avoid cross-socket memory latency
- `cpupower frequency-set -g performance` — disable frequency scaling
- Huge pages: `echo 512 > /proc/sys/vm/nr_hugepages`

**Warning from Jump:** reading `/proc/cpuinfo` from a monitoring
thread triggered IPIs on isolated cores. Telemetry must not touch
isolated cores. Route metrics via SPSC queue to a monitoring thread
on a non-isolated core.

**RSX gaps vs Jump:**
- IPI mitigation not yet documented (Gateway on Tokio can send IPIs
  to ME core)
- No interrupt affinity in deployment guide
- NUMA topology not explicitly specified in ops docs

## See Also

- `SPEEDUP.md` - Full bottleneck analysis and capacity estimates
- `DEPLOY.md` - Single-machine deployment target
- `rsx-risk/src/shard.rs` - Margin recalculation entry point
- `rsx-matching/src/snapshot.rs` - Snapshot save/load (scheduler
  pending)
- `specs/v1/TILES.md` - Tile architecture and SPSC ring layout
