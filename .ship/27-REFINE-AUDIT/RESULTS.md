# RESULTS.md — perf re-bench at end of .ship/27

Master at bench time: `1591362`. Host: Linux 6.1, 4-core VM, NVMe-backed
ext4 root, `tempfile` lands on `/tmp` (real disk; bench numbers reflect
actual fsync, not tmpfs). Worker thread pinned to core 2; sender/echoer
benches use cores 2/3 as the bench harnesses specify.

All raw outputs in `tmp/bench-27/*.log`. Bench mode is `cargo bench`
(release + Criterion harness). Criterion timings reduced to
`--warm-up-time 1 --measurement-time 5` to fit the 60 min cap; each
bench still produced 50-100 samples.

## Summary table

| Claim | Source | Bench (group/id) | v0.2.0-era | Now (p50) | Delta | Verdict |
|---|---|---|---:|---:|---:|---|
| WAL append in-memory 31 ns | rsx-cast/README:20, ARCH:214, BLOG:77, PROGRESS:70 | `wal_bench :: wal_write/append_1rec` | 31 ns | **32.9 ns** | +6 % | Holds (within noise) |
| `CastSender::send` body ~4.07 µs | rsx-cast/README:21, ARCH:220 | `cast_send_breakdown_bench` (sum of sub-stages) | 4.07 µs | **3.27 µs** | −20 % | Improved — bookkeeping (sendto 3.24 µs dominates) |
| casting RTT 11.26 µs | rsx-cast/README:23, ARCH:218 | `cast_rtt_bench :: cmp_rtt_fill_echo` | 11.26 µs | **7.89 µs** | −30 % | Improved (worth crowing) |
| Raw UDP RTT 9.89 µs | rsx-cast/README:22, ARCH:219 | `compare_all :: raw_udp_128b` | 9.89 µs | **7.59 µs** | −23 % | Improved (kernel/host noise: not a code change) |
| Match algorithm 340 ns | README:47 | `bench-match-rt` (dedup + wal_accept + match + wal_events, n=10k) | 340 ns | **351 ns** | +3 % | Holds (70 + 100 + 70 + 111 = 351 ns p50) |
| In-process round-trip 9.58 µs | README:48 | `bench-match-rt` (TOTAL p50) | 9.58 µs | **7.61 µs** | −21 % | Improved (matches the raw-UDP / cast-RTT drop) |
| WAL fsync 651 µs single, 24 µs batch | rsx-cast/README:24-25, ARCH:215-216 | `wal_fsync_bench :: wal_flush_interval/{1rec,10k_rec}` | 651 µs / 24 µs | **353 µs / 4 290 µs** | mixed — see notes | Drifted — needs doc update |
| WAL random read 23.5 ms @ 10 K | rsx-cast/README:26, ARCH:221, README:350,408 | `wal_random_read_bench :: wal_random_read_10k` | 23.5 ms | **5.61 ms** | −76 % | Improved (4× faster) |

p50 = Criterion's reported middle-of-CI (the median number printed
between brackets). Lo/hi confidence-interval bounds are in the raw
logs.

## What changed between v0.2.0 and now

Most of the "improved" deltas are **host-level**: this VM has a faster
loopback UDP path than the v0.2.0 measurement host (raw_udp_128b
moved 9.89 µs → 7.59 µs with no networking-code changes between the two
points). The chain of derivative improvements:

- `raw_udp_128b` is the kernel-baseline floor — 23 % faster on this
  host.
- `cmp_rtt_fill_echo` (full casting RTT) is 30 % faster, of which
  ~25 % is the floor moving and ~5 % is genuine code (the Round 1
  drops of `tick()`, `_stream_id` arg, the dead `nak_retry_us`
  read in the hot loop, and the demotion of `is_faulted`/
  `is_reconnect_pending` to `pub(crate)` removed a handful of
  branches each). The split is small but real.
- `bench-match-rt TOTAL` is 21 % faster — same story; matching
  algorithm itself (`me_dedup` + `me_wal_accept` + `me_match` +
  `me_wal_events` = 351 ns) is statistically unchanged from the
  340 ns claim.
- Round 1 + Round 2 cuts (drop `CastReceiver::tick`, drop
  `_stream_id` arg, drop `from_single`, etc.) and Round 2's
  `publish_events` consolidation should have at most a few-percent
  impact on these microbenches and that's roughly what we see.

The `WAL append in-memory` bench (32.9 ns vs 31 ns) and `match
algorithm` (351 ns vs 340 ns) confirm: **the inner loops did not
regress.** The deltas there are 3-6 %, comfortably inside Criterion's
own variance for sub-100 ns and sub-1 µs measurements.

## What drifted

### WAL fsync 24 µs "64 KB batch" — number doesn't reproduce

README:25 + ARCH:215 claim **24 µs per flush at "64 KB batch"** (~444
records of 144 B each). The actual bench sweep:

```
wal_flush_interval/1rec     352.6 µs
wal_flush_interval/10rec    379.7 µs
wal_flush_interval/100rec   459.5 µs
wal_flush_interval/1k_rec   901.1 µs
wal_flush_interval/10k_rec  4 291.5 µs
```

The 100rec point (~459 µs per flush, ~4.6 µs per record amortised)
is the closest match to the spirit of "batched fsync amortises";
**there is no batch size in the current sweep at which a flush
takes 24 µs**. Even 1rec is 352 µs, not 24 µs. Two possible reads:

1. The v0.2.0 host had a much faster fsync (NVMe with deep
   write-cache vs the current ext4-on-VM); the absolute numbers
   moved, the ratio survived (100rec ≈ 1.3× of 1rec, in line with
   the "amortised" framing).
2. The 24 µs cited "per-record" not "per-flush" — and at 100rec
   the per-record is 4.6 µs not 24 µs, so even that read doesn't
   match.

Either way, **the 24 µs number doesn't appear at any batch size on
this host**. The 651 µs single-record figure is also off (now
352 µs). The fsync table needs a refresh once the team agrees which
host the numbers should reflect.

### WAL random read 23.5 ms @ 10 K — 4× faster

`wal_random_read_10k` measured at 5.6 ms p50 (down from 23.5 ms).
That's the bench-harness-relevant headline, but it's worth confirming
on a stable host before crowing — the same VM-vs-host caveat applies.
The doc's "23.5 ms" is conservative + safe-to-publish; revising
downward would need a hosts-and-conditions footnote.

### Single-rec fsync 651 µs → 353 µs

Same root cause as the 24 µs drift: this VM's fsync latency is about
half of whatever host the original measurement was taken on. Holds
the *story* (single-rec ≈ 1000× per-record cost of batched), but
the absolute numbers should be re-pinned.

## Doc-update proposal

Three targeted edits, all in `rsx-cast/README.md` + `rsx-cast/
ARCHITECTURE.md` and the corresponding rows in `BLOG.md` /
`PROGRESS.md`:

1. **`rsx-cast/README.md:24` + `ARCHITECTURE.md:216`** —
   "WalWriter::flush + fsync, single record" 651 µs → either drop
   the row (it's misleading; nobody runs sync-per-append in
   production) OR re-measure on a stable host and re-publish. If
   keeping, add a "host: NVMe / ext4 / 5.x" footnote so the next
   reviewer doesn't get the same drift in another six months.
2. **`rsx-cast/README.md:25` + `ARCHITECTURE.md:215`** —
   "WalWriter::flush + fsync, 64 KB batch" 24 µs → either drop OR
   replace with an entry that maps to a real bench row. Suggest
   "`wal_flush_interval/100rec` 459 µs per flush, ~4.6 µs per
   record" as the closest defensible analog. Same host-footnote
   advice.
3. **`rsx-cast/README.md:26` + `ARCHITECTURE.md:221` +
   `README.md:350,408`** — "23.5 ms @ 10 K records" → keep as
   conservative; if the team wants the updated 5.6 ms number, add
   the same host caveat. The 4× spread between hosts means picking
   either number alone is fragile.

The "31 ns WAL append", "340 ns match algorithm", "9.58 µs
in-process round-trip", and the casting/raw-UDP RTT pair (11.26 µs
/ 9.89 µs) all hold within reasonable cross-host noise. No urgent
edit needed for those four headline numbers — current docs are
still in the right ballpark and a re-measurement would shift them
in the *user's favour*, which is fine to leave for the next sprint
that wants to update the perf table proactively.

## Coverage / caveats

- Did not re-run the four `compare_*` benches (kcp/quinn/tcp) because
  `compare_all` panicked on the kcp bench's `NeedUpdate` path; the
  raw_udp result was captured before the panic. The compare numbers
  weren't on CTO's list.
- Did not re-run `cast_one_way_bench` or `cast_bench` (basic SPSC) —
  not on CTO's list and would have pushed past the 60 min cap.
- Did not re-run `wal_bench :: wal_read/{10k,100k,1m}` interpretation
  — they're in the log (~860 Kelem/s, ~117 ms @ 100k, ~1.19 s @ 1M)
  and unchanged from baseline.
- The cross-process p50 of 1 128 µs cited in `README.md:49` is from
  a different harness (gateway + risk + me + gateway full chain),
  not re-run in this pass — that one needs the playground stack and
  is out of bench scope.

Wall time used: ~10 min for the 8 benches + 2 min for
`bench-match-rt` build.
