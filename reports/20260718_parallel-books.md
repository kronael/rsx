# One-book and parallel-book crossing load — 2026-07-18

Commit: `309354240c467c852b2e810a0971587f1bb3dfbe`

Host: `hel1v2`, Linux 6.1.0-43-amd64, KVM guest, 8 vCPUs from an AMD Ryzen
9 5950X, 7.8 GiB RAM, no swap. Gateway, risk, matching engines, market data,
maker, Playground, and all 20 Python load processes shared this host. Server
timestamp legs are comparable only because every process used the same host
clock. These are debug-build, shared-host results, not bare-metal or capacity
claims.

Each measurement started after `./rsx-playground/playground reset --yes` and a
fresh scenario boot. The public Make target was unchanged.

## One book: valid canonical run

```sh
MODE=single RATE=1750 N=3500 MIN_SAMPLES=3500 \
  USERS=1000 CONNECTIONS=1000 PROCESSES=20 \
  RSX_STRESS_SYMBOLS=10 make perf-load
```

Topology: minimal scenario, one PENGU matching engine, 1,000 WebSockets. The
two-second open-loop window offered 3,520 orders; `N=3500` is the validation
floor rather than an exact order limit.

- Offered/submitted/accepted/completed/filled: 3,520 each
- Rejected/timed out/pending/errors/send errors: 0 each
- Achieved: 1,742.57 orders/s, 99.58% of the 1,750/s target
- Client RTT: p50 1,427 us; p99 25,423 us; p99.9 31,077 us; max 35,094 us

| Server leg | Samples | p50 ns | p99 ns | max ns |
|---|---:|---:|---:|---:|
| gateway to risk | 3,520 | 16,125 | 5,404,605 | 10,109,164 |
| risk to matching | 3,520 | 24,296 | 3,033,660 | 7,009,589 |
| matching | 3,520 | 11,827 | 121,393 | 2,099,218 |
| match done to gateway | 3,520 | 216,000 | 16,457,491 | 22,808,416 |
| total internal | 3,520 | 318,332 | 17,947,607 | 23,539,628 |

This valid result is the final `bench-e2e-latest.json` shape and supplied the
explicitly sealed `bench-reference.json` via
`SKIP_RUN=1 MIN_SAMPLES=3500 make perf-e2e-save`.

## Three books: requested rate missed

```sh
MODE=single RATE=5250 N=10500 MIN_SAMPLES=10500 \
  USERS=1000 CONNECTIONS=1000 PROCESSES=20 \
  RSX_STRESS_SYMBOLS=10,3,1 make perf-load
```

Topology: trio scenario, independent PENGU/SOL/BTC matching engines, shared
risk and gateway, 1,000 WebSockets. The workload requested the same 1,750
orders/s per book. It was run once and not tuned or repeated after failure.

- Offered/submitted/accepted/completed/filled: 10,500 each
- Rejected/timed out/pending/errors/send errors: 0 each
- Achieved: 4,646.02 orders/s, 88.50% of the 5,250/s target
- Client RTT: p50 252,066 us; p99 514,259 us; p99.9 583,242 us; max 610,453 us
- Result: invalid for publication because the achieved/offered ratio was below
  the existing 95% floor, despite closed and lossless accounting

| Symbol | Offered/completed/filled | Client p99 us | Internal p50/p99 ns | Matching p50/p99 ns |
|---|---:|---:|---:|---:|
| PENGU (10) | 3,507 | 517,370 | 26,476,898 / 155,559,855 | 9,237 / 111,059 |
| SOL (3) | 3,496 | 513,690 | 28,129,803 / 156,093,361 | 9,468 / 130,782 |
| BTC (1) | 3,497 | 514,217 | 26,851,400 / 153,469,849 | 9,237 / 115,529 |

| Aggregate server leg | Samples | p50 ns | p99 ns | max ns |
|---|---:|---:|---:|---:|
| gateway to risk | 10,500 | 129,082 | 9,513,134 | 22,581,780 |
| risk to matching | 10,500 | 341,656 | 17,640,730 | 20,184,774 |
| matching | 10,500 | 9,308 | 118,896 | 16,934,007 |
| match done to gateway | 10,500 | 22,980,428 | 154,069,649 | 160,467,819 |
| total internal | 10,500 | 27,083,340 | 155,274,826 | 160,949,603 |

All three books did equal work and their matching medians stayed near 9 us.
The miss accumulated predominantly after match completion and in external
client RTT. This demonstrates functioning independent-book fan-out, but does
not establish 5,250 orders/s as a stable shared-host rate or imply linear
hardware scaling.
