# Cross-language baselines — cited context, NOT benched

These are public numbers from other projects' own harnesses, on their own
hardware, in a different language. They are **not** re-run here — quoted
with full caveats, never presented as a head-to-head against rsx-book's
numbers. A same-box rebuild of any of these (cloning the project, building
it with its own bench flags, running it on the box this repo's Criterion
numbers come from) is a real, larger stretch goal, not attempted in this
pass. See `.ship/34-COMPARE-RESEARCH/PLAN.md` for the full research this is
distilled from.

## charles-cooper/itch-order-book (C++)

- **Number:** 61 ns/tick, 16M msgs/s.
- **HW/workload:** 2012-era Intel i7-3820, single core, real Nasdaq
  TotalView-ITCH sample file replay (Add/Execute/Cancel/Replace ticks),
  warmed page cache.
- **Language/design:** C++, flat vectors/arrays — no hashmap, no tree,
  single symbol.
- **NOT apples-to-apples because:** different language (C++ vs Rust),
  ~14-year-old hardware (2012 vs whatever box this repo's numbers run on —
  disclose per-number HW always), and it's parse + book-maintenance, not a
  matching *engine* (no fill generation) — the fairer rsx-book line to
  mentally place it next to is `insert_resting_order`/`cancel_order`, not
  `match_*`.
- **What would make it fair:** clone `itch-order-book`, build with its own
  bench flags, get a real public Nasdaq ITCH sample
  (`https://emi.nasdaq.com/ITCH/Nasdaq%20ITCH/`), run both it and an
  ITCH-replay-adapter into rsx-book on the SAME box. Removes the HW
  variable entirely — the single most bulletproof comparison available in
  the whole research set, but a real build-and-adapter-writing task, not
  attempted here.

## exchange-core (Java, LMAX-Disruptor-based)

- **Number:** detailed latency percentiles — 1M ops/s: p50 0.5µs / p99 4µs
  / p99.99 31µs; 5M ops/s: p50 1.5µs / p99 42µs. Separately, a headline
  claim of "~150ns per matching for large market orders."
- **HW/workload:** Intel Xeon X5690 (2010-era, 3.47GHz), single socket,
  isolated + tickless, Spectre/Meltdown mitigations OFF, a specific
  workload mix (9% GTC / 3% IOC / 6% cancel / 82% move / ~6% trigger),
  1000 accounts × ~1000 orders across ~750 price slots, 3M messages.
- **Language/design:** Java, JVM, LMAX Disruptor ring-buffer core.
- **NOT apples-to-apples because:** JVM (GC pauses, JIT warmup — a
  fundamentally different latency distribution shape than a native Rust
  bench with no GC), 2010-era HW with mitigations disabled (not
  representative of a modern production box), percentile-based own-harness
  measurement (not Criterion, not independently reproduced here), and the
  "~150ns per matching" line is an isolated micro-claim with no visible
  methodology alongside it.
- **Usable as:** order-of-magnitude sanity check only — "a serious
  production-grade engine's own micro-claim sits in the same 100-200ns
  neighborhood as rsx-book's `match_*` benches" — never as a head-to-head
  ns comparison.

## liquibook (enewhuis/liquibook, C++)

- **Number:** "2.0-2.5M inserts/sec" (README's own words).
- **HW/workload:** none disclosed beyond "varies by HW/OS" in the README.
- **Language/design:** header-only C++ matching engine, full order
  lifecycle (accept/reject/fill/cancel/replace + depth book).
- **NOT apples-to-apples because:** aggregate throughput number, no
  percentile, no HW spec, no isolation of insert-only vs full
  accept-with-matching. A derived "~400-500 ns/insert" figure would be
  **our own arithmetic**, not liquibook's claim — if ever quoted, it must
  be labeled as a derivation, not represented as something liquibook
  reported.
- **Usable as:** "the other well-known open-source matching engine reports
  a number in this ballpark" context, with heavy caveats — not a number.

## Why these three and not others

`LMAX Disruptor`'s own micro-benchmarks (52ns/hop ring-buffer numbers) and
`OrderBook-rs`'s concurrent-throughput numbers are excluded even as cited
context — they measure a different layer (messaging) or a different axis
(contended throughput vs uncontended latency) respectively. See
`.ship/34-COMPARE-RESEARCH/PLAN.md`'s fairness guardrails section and
`compare/orderbook-rs.md` for the full reasoning.
