# Book and matching publication benchmark — 2026-07-16

Quiet-host rerun of the two depth-invariance demos and the matching tile's
accept/throughput benchmark. These are publication numbers for the current
code, not replacements for end-to-end exchange latency measurements.

## Results

`rsx-book` — `deep_flat_match`, one non-draining match:

| resting orders | median | Criterion interval |
|---:|---:|---:|
| 100,000 | 70.239 ns | 69.099–71.521 ns |
| 1,000,000 | 70.600 ns | 69.491–71.854 ns |
| 10,000,000 | 69.197 ns | 68.396–70.047 ns |

`rsx-matching` — `match_by_depth`, one qty-1 non-draining partial fill:

| resting orders | median | Criterion interval |
|---:|---:|---:|
| 1 | 46.343 ns | 45.257–47.490 ns |
| 100 | 48.536 ns | 47.309–49.857 ns |
| 1,000 | 49.136 ns | 48.334–50.111 ns |
| 10,000 | 48.951 ns | 48.154–49.872 ns |
| 100,000 | 49.491 ns | 48.811–50.204 ns |

The matching tile's full one-fill accept path measured 256.25 ns
(246.16–266.57 ns). The throughput fixture measured 3.9568 million orders/s
(3.8479–4.0760 million/s).

## Method

- Host: shared KVM guest, AMD Ryzen 9 5950X, 8 vCPUs.
- Toolchain: `rustc 1.95.0-nightly (eda76d9d1 2026-01-21)`, LLVM 21.1.8.
- The RSX cluster was stopped and the host was otherwise quiet.
- Criterion 0.5.1, optimized bench profile, 3 s warm-up, 50 samples.
- In-process, closed-loop microbenchmarks; no UDP, WebSocket, cross-process
  scheduling, persistence flush, or network round trip.
- Source tree: `dc635f7` (includes book reclamation commits `a01e25c` and
  `dc635f7`, and matching stale-slot fix `11abb23`).

Commands:

```sh
cargo bench -p rsx-book --bench deep_book_bench -- deep_flat_match
cargo bench -p rsx-matching --bench match_depth_bench
cargo bench -p rsx-matching --bench process_order_bench -- \
  'me_accept_path/full|me_throughput/orders'
```

Captured command output is in `tmp/20260716-book-demo-bench.log`,
`tmp/20260716-matching-depth.log`, and
`tmp/20260716-matching-accept.log`. `tmp/` is transient; this report is the
durable record.

The final GIF capture reran the same live benches and displayed rounded
medians of 70–71 ns for book and 54–58 ns for matching. The difference from
the tables above is shared-host run variance; both captures preserve the same
bounded depth shape.

## Conclusion and limits

Both depth sweeps remain flat: book depth from 100,000 to 10 million and
matching-tile depth from 1 to 100,000 do not add a growing latency term. Exact
nanoseconds vary with host load and hardware. Quote these as dated compute
floors, never as exchange latency or production throughput.
