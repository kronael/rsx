# Ten-minute crossing-load tail — 2026-07-18

Commit: `309354240c467c852b2e810a0971587f1bb3dfbe`

```sh
MODE=long RATE=1750 LONG_DURATION=600 REPEATS=1 \
  N=3500 MIN_SAMPLES=3500 LONG_MIN_SAMPLES=3500 \
  USERS=1000 CONNECTIONS=1000 PROCESSES=20 \
  RSX_STRESS_SYMBOLS=10 make perf-load
```

The run used the same minimal one-book topology as the canonical short run:
one PENGU matching engine, crossing IOC orders, 1,000 users, 1,000 WebSockets,
and 20 Python processes. It began from reset state and ran once as configured.

- Duration: 600.04 s
- Offered/submitted/accepted/completed/filled: 1,050,020 each
- Rejected/timed out/pending/errors/send errors: 0 each
- Achieved: 1,749.92 orders/s, 100.00% of the 1,750/s target after rounding
- Client RTT: p50 1,651 us; p99 100,617 us; p99.9 181,708 us; max 290,111 us

| Server leg | Samples | p50 ns | p99 ns | max ns |
|---|---:|---:|---:|---:|
| gateway to risk | 1,050,020 | 17,082 | 5,308,594 | 32,242,112 |
| risk to matching | 1,050,020 | 30,046 | 6,557,666 | 118,333,029 |
| matching | 1,050,020 | 10,900 | 92,253 | 118,342,817 |
| match done to gateway | 1,050,020 | 284,594 | 38,302,910 | 125,860,901 |
| total internal | 1,050,020 | 421,706 | 43,470,603 | 128,294,195 |

Accounting and timestamp completeness closed for all 1,050,020 orders. The
tail is observational: p99.9 and max are not regression gates. Gateway, risk,
matching, market data, maker, Playground, and the load generator shared an
8-vCPU KVM guest (AMD Ryzen 9 5950X host CPU, 7.8 GiB RAM). Server legs share
one host clock; client RTT additionally contains Python scheduling, WebSocket
serialization, socket delivery, and response draining. This is debug-build
shared-host evidence, not a bare-metal latency claim.
