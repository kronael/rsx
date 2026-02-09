# Knowledge Base: Industry Findings & Design Validation

## Source: LWN Article on HFT and Linux Networking

**Reference:** https://lwn.net/Articles/914992/
**Author:** PJ Waskiewicz (Jump Trading)
**Event:** Netdev 0x16
**Date Analyzed:** 2024-02-07

---

## Key Findings from Industry (Jump Trading)

### 1. Latency Requirements

**Industry Priority:** Predictable latency (jitter reduction) > raw speed

**Quote:** "Network jitter can cause HFT firms to lose a lot of money"

**RSX Alignment:**
- ✅ ORDERBOOK.md: Single-threaded matching (no lock jitter)
- ✅ SMRB.md: Core pinning for predictable execution
- ✅ FUTURE.md: Userspace networking to eliminate kernel jitter
- ⚠️ NETWORK.md: QUIC + io_uring reduces jitter vs epoll, still has
  kernel latency variance

**Action:** v2 should prioritize jitter reduction over average latency.

### 2. Performance Benchmarks (Jump Trading)

**Latency measurements (synthetic):**
- Baseline: 51.6µs min, 68.7µs mean
- CPU pinning + interrupt affinity: 45.4µs min, 53.1µs mean
- Polling mode (no interrupts): 41.9µs min, 56.3µs mean

**RSX Current Targets (NETWORK.md):**
- QUIC over network: ~10-50µs (inter-process)
- SMRB (future): ~50-200ns (same machine)

**Analysis:**
- ✅ RSX targets are realistic (QUIC faster than Jump's baseline)
- ✅ Jump's 41.9µs includes network stack (full round-trip)
- ✅ RSX 10-50µs is QUIC IPC (comparable to optimized kernel)
- ✅ RSX v1 is competitive with Jump's optimized baseline

**RSX Future Targets (FUTURE.md):**
- Raw structs over SMRB: ~50-200ns (IPC only, not network)
- Userspace networking: ~1-5µs (with DPDK)

**Analysis:**
- ✅ SMRB target (50-200ns) is for IPC, not comparable to Jump's network latency
- ⚠️ Userspace networking target (1-5µs) is 40x faster than Jump's results
- ❓ Need to clarify: are we measuring same thing? (IPC vs network vs end-to-end)

### 3. Industry-Standard Solution (From Article Comments)

**Reality Check:** Solarflare NICs with kernel bypass achieve ~30ns tick-to-trade

**Comparison:**
- Industry standard: ~30ns (specialized hardware)
- Jump Trading: ~41.9µs (optimized Linux)
- RSX v1 (QUIC): ~10-50µs (inter-process IPC)
- RSX v2 (SMRB): ~50-200ns (same-machine IPC)

**Gap Analysis:**
- Industry (Solarflare): 30ns **end-to-end** (network → matching → network)
- RSX v1 (QUIC): 10-50µs **IPC** (Gateway → Matching via network stack)
- RSX v2 (SMRB): 50-200ns **IPC only** (Gateway → Matching, same machine)
- Missing: full end-to-end includes network ingress/egress

**Conclusion:** RSX v1 QUIC is competitive with Jump's baseline. v2 SMRB
optimization addresses same-machine IPC. To compete with Solarflare-class
systems, need userspace networking (DPDK/XDP) + specialized NICs.

### 4. Kernel Bypass Technologies

**Jump Trading Interest:** XDP/AF_XDP for selective kernel bypass

**RSX Documentation:**
- v1 uses io_uring (monoio) for async I/O with kernel batching
- FUTURE.md mentions: DPDK, AF_XDP for full kernel bypass
- Categorized as "extreme optimization" (not v1, not v2)

**Alignment:**
- ✅ Jump Trading validates need for kernel bypass (big firms exploring)
- ✅ RSX v1 uses io_uring (better than epoll, not full bypass)
- ⚠️ Article suggests XDP/AF_XDP is emerging (not bleeding edge)

**Action:** Consider XDP/AF_XDP for v3 (before full DPDK commitment).

### 5. CPU Isolation & Interrupt Affinity

**Jump Trading Techniques:**
- `isolcpus` boot parameter (remove CPUs from scheduler)
- NUMA locality management
- Interrupt affinity (pin IRQs to specific cores)
- Polling mode (eliminate interrupt jitter)

**RSX Documentation:**
- SMRB.md: "Core pinning: Pin producer and consumer to specific cores"
- ORDERBOOK.md: "Dedicated core (pinned, no context switches)"
- FUTURE.md: Mentions DPDK (implies polling mode)

**Alignment:**
- ✅ RSX design includes core pinning (mentioned in SMRB.md)
- ⚠️ No explicit mention of `isolcpus`, NUMA, interrupt affinity
- ⚠️ No deployment guide for production tuning

**Gap:** Need operational guide for:
- Boot parameters (`isolcpus=`, `nohz_full=`)
- IRQ affinity configuration (`/proc/irq/*/smp_affinity`)
- NUMA binding (`numactl`)
- Huge pages configuration

### 6. Jitter Sources

**Jump Trading Findings:**
- Unexpected IPIs (inter-processor interrupts) despite isolation
- TLB shootdowns (even on isolated cores)
- Bug: Reading `/proc/cpuinfo` triggered IPIs (telemetry impact)

**RSX Implications:**
- ⚠️ Single-threaded matching (ORDERBOOK.md) avoids cross-core IPIs
- ⚠️ But Gateway is multi-threaded (Tokio) - may send IPIs to Matching core
- ❌ No mention of IPI mitigation in RSX docs

**Action:**
- Isolate Matching Engine core completely (no other threads)
- Gateway runs on separate cores (NUMA node if possible)
- Avoid any syscalls on Matching core (no logging, no metrics polling)

### 7. Network Protocols

**Jump Trading Setup:**
- 10Gbps Ethernet to exchanges (Nasdaq, Eurex, KRX)
- Proprietary protocols (each exchange different)
- RDMA over Infiniband/RoCE for internal HPC

**RSX Design:**
- NETWORK.md: WebSocket/REST (external), QUIC + WAL wire format (internal)
- FUTURE.md: Mentions raw structs over SMRB (same machine only)

**Alignment:**
- ⚠️ RSX has no plan for RDMA (Jump uses for internal HPC)
- ✅ RSX uses WebSocket/binary protocols (exchange-compatible)
- ✅ RSX correctly focuses on same-machine first (SMRB before network)

**For RSX:** Exchanges use FIX, WebSocket, or proprietary binary. RSX v1
uses WebSocket for external, QUIC for internal. External adapter layer
for FIX if needed.

### 8. Infrastructure Challenges

**Jump Trading:**
- Multiple exchanges with different protocols (integration complexity)
- Tens to hundreds of thousands of CPUs for quant analysis
- Telemetry causes jitter (monitoring is a problem)

**RSX Scope:**
- Building the exchange itself (not connecting to external exchanges)
- Much smaller scale (not HPC cluster)
- No mention of monitoring/telemetry impact on latency

**Gap:** RSX needs telemetry design that doesn't impact hot path:
- Separate monitoring core (reads shared memory)
- No syscalls on Matching core (no disk I/O, no network stats)
- Export metrics via lock-free queue (SPSC to monitoring thread)

---

## Design Validation: RSX vs Jump Trading

### What RSX Got Right

1. **Single-threaded matching** (ORDERBOOK.md)
   - Aligns with CPU isolation approach
   - Avoids lock jitter, IPI jitter

2. **Core pinning** (SMRB.md)
   - Mentioned for SPSC producers/consumers
   - Jump validates this is critical

3. **Deferred userspace networking** (FUTURE.md)
   - Correctly categorized as future optimization
   - Jump shows even big firms still exploring (not premature)

4. **SMRB for same-machine** (FUTURE.md)
   - Correct prioritization (optimize local first)
   - Jump uses RDMA for cross-machine (validates need for fast IPC)

5. **Fixed-point math** (ORDERBOOK.md)
   - No mention in Jump article, but standard industry practice
   - Avoids floating-point non-determinism

### What RSX Missed

1. **Jitter as primary concern**
   - RSX focuses on average latency, not tail latency or jitter
   - Need p50, p99, p99.9 measurements (not just mean)

2. **IPI mitigation**
   - No mention of isolating Matching core from other threads
   - Gateway (Tokio, multi-threaded) may cause IPIs

3. **Interrupt affinity**
   - Not mentioned in any RSX doc
   - Critical for predictable latency

4. **NUMA awareness**
   - Not mentioned in RSX docs
   - Jump emphasizes this for multi-socket systems

5. **Telemetry impact**
   - No design for monitoring without jitter
   - Jump found `/proc/cpuinfo` reads caused IPIs

6. **End-to-end latency budget**
   - RSX measures IPC latency, not full user → matching → user
   - Need to account for: network ingress, Gateway processing, IPC,
     Matching, IPC back, Gateway processing, network egress

### What RSX Should Add

1. **OPERATIONS.md** - Production deployment guide
   - Boot parameters: `isolcpus`, `nohz_full`, `rcu_nocbs`
   - IRQ affinity: Pin NIC interrupts to non-matching cores
   - NUMA binding: Gateway and Matching on same socket
   - Huge pages: Reduce TLB misses
   - CPU governor: `performance` (no frequency scaling)

2. **MONITORING.md** - Telemetry without jitter
   - Lock-free stats export (SPSC queue to monitoring thread)
   - No syscalls on Matching core (no file I/O, no network)
   - Separate monitoring process reads shared memory

3. **LATENCY.md** - Latency budget breakdown
   - End-to-end: User → Gateway → Matching → Gateway → User
   - Budget per component: Network (??), Gateway (??), IPC (??), Matching (??)
   - Target percentiles: p50, p99, p99.9, p99.99
   - Jitter tolerance: max acceptable deviation

4. **Update FUTURE.md** - XDP/AF_XDP before DPDK
   - Jump shows XDP/AF_XDP is emerging (not bleeding edge)
   - Less invasive than full DPDK (selective bypass)
   - io_uring integration (Jump mentioned this)

---

## Performance Reality Check

### Latency Comparison Table

| System | Latency | Scope | Notes |
|--------|---------|-------|-------|
| **Industry (Solarflare)** | ~30ns | End-to-end (tick-to-trade) | Specialized hardware, kernel bypass |
| **Jump (optimized Linux)** | ~42µs | Network round-trip | Synthetic benchmark, optimized kernel |
| **Jump (baseline Linux)** | ~52µs | Network round-trip | Standard kernel, no tuning |
| **RSX v1 (QUIC)** | ~10-50µs | IPC (Gateway ↔ Matching) | QUIC + io_uring, kernel networking |
| **RSX v2 (SMRB)** | ~50-200ns | IPC only (Gateway ↔ Matching) | Shared memory, no serialization |
| **RSX v3 (DPDK)** | ~1-5µs | Network + IPC (estimated) | Userspace networking, full stack |

### Critical Insight

**RSX v1 IPC (10-50µs) is faster than Jump's baseline (52µs)**

RSX v1 **internal communication** via QUIC + io_uring is competitive
with Jump's optimized network stack. QUIC provides lower latency than
traditional TCP/epoll.

**Benefits:** QUIC + WAL wire format (fixed #[repr(C)] structs)
provides both speed and reliability.

**Future:** Move to SMRB (v2) for same-machine (50-200ns), use QUIC
for cross-machine communication.

### Realistic RSX End-to-End Budget (v2)

```
User (internet) → Gateway → Matching → Gateway → User

Network ingress:    ~1,000 µs  (internet, variable)
TLS termination:       ~100 µs  (handshake amortized)
JSON parse:            ~10 µs   (serde_json)
Gateway validation:    ~1 µs    (risk check, margin calc)
IPC (Gateway→Match):   ~0.1 µs  (SMRB, 100ns)
Matching:              ~0.5 µs  (O(1) orderbook ops)
IPC (Match→Gateway):   ~0.1 µs  (SMRB, 100ns)
JSON serialize:        ~5 µs    (serde_json)
Network egress:        ~1,000 µs (internet, variable)
────────────────────────────────
Total:                 ~2,117 µs (2.1ms)
```

**Dominated by network (internet).** Internal optimization (SMRB) only saves
~10-50µs vs QUIC. Real win is eliminating internet latency via co-location.

### Co-Located Client (Same Data Center)

```
Client (1Gbps LAN) → Gateway → Matching → Gateway → Client

Network ingress:    ~0.1 µs  (LAN, low jitter)
JSON parse:         ~10 µs
Gateway validation: ~1 µs
IPC (G→M):          ~0.1 µs  (SMRB)
Matching:           ~0.5 µs
IPC (M→G):          ~0.1 µs  (SMRB)
JSON serialize:     ~5 µs
Network egress:     ~0.1 µs  (LAN)
────────────────────────────────
Total:              ~17 µs
```

**Now JSON is bottleneck.** Move to binary protocol (FlatBuffers or raw structs).

### Co-Located Client + Binary Protocol

```
Client (binary) → Gateway → Matching → Gateway → Client

Network ingress:    ~0.1 µs  (LAN)
Binary parse:       ~0.05 µs (zerocopy)
Gateway validation: ~1 µs
IPC (G→M):          ~0.1 µs  (SMRB)
Matching:           ~0.5 µs
IPC (M→G):          ~0.1 µs  (SMRB)
Binary serialize:   ~0.05 µs (zerocopy)
Network egress:     ~0.1 µs  (LAN)
────────────────────────────────
Total:              ~2 µs
```

**Still 2µs (2000ns), 67x slower than Solarflare (30ns).**

**Remaining bottleneck:** Network stack (kernel, TCP/IP, driver).

### Co-Located Client + DPDK

```
Client (DPDK) → Gateway → Matching → Gateway → Client

Network ingress:    ~0.005 µs (5ns, DPDK)
Binary parse:       ~0.050 µs (50ns)
Gateway validation: ~1.000 µs (1000ns, still slow)
IPC (G→M):          ~0.100 µs (100ns, SMRB)
Matching:           ~0.500 µs (500ns, orderbook)
IPC (M→G):          ~0.100 µs (100ns, SMRB)
Binary serialize:   ~0.050 µs (50ns)
Network egress:     ~0.005 µs (5ns, DPDK)
────────────────────────────────
Total:              ~1.8 µs (1800ns)
```

**Still 60x slower than Solarflare.**

**Remaining bottleneck:** Gateway validation (1µs) and Matching (500ns).

### Absolute Minimum (No Gateway, Matching Only)

```
Client (DPDK) → Matching Engine → Client

Network ingress:    ~0.005 µs (5ns)
Matching:           ~0.500 µs (500ns, orderbook O(1))
Network egress:     ~0.005 µs (5ns)
────────────────────────────────
Total:              ~510ns
```

**Still 17x slower than Solarflare (30ns).**

**Remaining bottleneck:** Orderbook matching (500ns).

### To Reach 30ns (Solarflare-Class)

Need:
- FPGA or ASIC (not CPU)
- Orderbook in hardware (parallel matching)
- No software in critical path

**Conclusion:** RSX on CPU will never reach 30ns. Target should be ~500ns-2µs
(Matching only) or ~2-10µs (Gateway + Matching). This is still competitive for
most use cases (retail, non-HFT market making).

---

## Recommendations

### Immediate (v1)

1. **Add latency budget to NETWORK.md**
   - Document end-to-end latency (not just IPC)
   - Clarify what 50-100µs refers to (IPC, not full round-trip)

2. **Add jitter to design goals**
   - ORDERBOOK.md: Add "predictable latency" as goal
   - Measure p99, p99.9 (not just mean)

3. **Document operational requirements**
   - Core pinning strategy (which cores for Gateway, Matching, Monitoring)
   - NUMA topology (same socket for Gateway + Matching)

### Short-Term (v2)

1. **Evaluate SMRB for same-machine deployments**
   - QUIC adds 10-50µs (acceptable for distributed systems)
   - SMRB adds 100ns (50-500x faster for same-machine)

2. **Create OPERATIONS.md**
   - Boot parameters, IRQ affinity, NUMA binding
   - Based on Jump Trading's techniques

3. **Create MONITORING.md**
   - Telemetry design that doesn't cause jitter
   - Lock-free stats export

4. **Add LATENCY.md**
   - End-to-end latency budget
   - Per-component breakdown
   - Target percentiles (p50, p99, p99.9)

### Long-Term (v3)

1. **Evaluate XDP/AF_XDP**
   - Jump shows this is emerging (not bleeding edge)
   - Before full DPDK commitment

2. **Benchmark against realistic targets**
   - Not Solarflare (30ns, FPGA)
   - Target: 2-10µs end-to-end for co-located clients

3. **Optimize Gateway validation**
   - Currently estimated 1µs (slow)
   - Consider pre-computed margin tables, SIMD

---

## Action Items

- [ ] Update NETWORK.md: Clarify latency measurements (IPC vs end-to-end)
- [ ] Update ORDERBOOK.md: Add "predictable latency" to design goals
- [ ] Create OPERATIONS.md: Production deployment guide
- [ ] Create MONITORING.md: Telemetry without jitter
- [ ] Create LATENCY.md: End-to-end latency budget
- [ ] Update FUTURE.md: Add XDP/AF_XDP (before DPDK)
- [ ] Benchmark SMRB vs QUIC (validate 50-500x speedup claim)
- [ ] Profile Gateway validation (is 1µs realistic?)
- [ ] Test on real hardware (measure actual jitter, not synthetic)

---

**Conclusion:** RSX design is fundamentally sound but missing operational details
for production deployment. Jump Trading article validates core choices (single-
threaded, core pinning, kernel bypass) but highlights gaps (IPI mitigation,
interrupt affinity, NUMA awareness, telemetry impact). RSX v1 with QUIC +
io_uring (10-50µs IPC) is competitive with Jump's optimized baseline (42µs full
network). SMRB (v2) optimization targets same-machine deployments (50-200ns).
