# Book fixed-work match distribution — 2026-07-27

## Measurement

`rsx-book` benchmark `deep_match_tail`: one non-draining partial fill while
the number of resting orders changes. Each sample is the average of 64
consecutive operations to amortize the timer.

| resting orders | p50 | p50 standard error | p99 |
|---:|---:|---:|---:|
| 100,000 | 52.594 ns | 0.024 ns | 87.656 ns |
| 1,000,000 | 50.719 ns | <0.001 ns | 82.969 ns |
| 10,000,000 | 53.688 ns | 0.002 ns | 111.922 ns |

## Method

- Source benchmark: `rsx-book/benches/deep_match_tail.rs`
- Source commit: `56b6ca4`
- Toolchain: `rustc 1.95.0-nightly (eda76d9d1 2026-01-21)`, LLVM 21.1.8
- Host: AMD Ryzen 9 5950X KVM guest
- Sampling: 20,000 warm-up operations, 31,250 batches, 64 operations per batch
- p50 standard error: 500 deterministic bootstrap resamples of batch averages

## Conclusion

The p50 range is 50.719–53.688 ns. The p99 range is 82.969–111.922 ns.
The 10-million-order case has the highest tail in this run.

## Caveats

Single core, in-process, closed-loop, shared host. Batch percentiles expose
slow batches, not isolated single-operation spikes. Setup time and memory
growth are outside the timed operation.
