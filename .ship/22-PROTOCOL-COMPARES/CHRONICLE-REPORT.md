# Chronicle Queue compare entry — final report

Last protocol entry in the `rsx-dxs/compare/` survey. The
seed file (72 LOC) was expanded to a full peer comparison;
no benchmark code was written.

## Option chosen: B (doc-only)

Brief defaulted to B and B is the right answer.

- No Rust client for Chronicle Queue exists. JNI would
  dominate any measurement.
- A Java JMH bench would measure a different thing
  (Java mmap IPC vs Rust UDP RTT), so the two numbers
  next to each other invite the wrong conclusion.
- The honest comparison is feature-axis and published-
  numbers, with each system's own benches doing the
  measuring of itself.

The doc says all of this in the "Why we did not write a
direct benchmark" section.

## What changed in `rsx-dxs/compare/chronicle-queue.md`

Seed was 72 LOC. Final is ~290 LOC.

Sections added or rewritten:
- **Why we include it** — frames Chronicle as the WAL+
  persistence-as-protocol peer (the new axis), not the UDP
  peer (Aeron's axis).
- **Design / Storage** — `.cq4` files, cycles, multi-level
  index, `metadata.cq4t` table store (with the v4→v5
  history of `directory-listing.cq4t`).
- **Design / Wire format** — Chronicle Wire (self-
  describing) + BytesMarshallable / FIELDLESS_BINARY
  (lower-level escape hatch).
- **Design / IPC** — steady-state-qualified statement
  about kernel transitions.
- **Design / Durability** — explicit defaults-vs-
  capabilities framing; OSS has `ExcerptCommon.sync()`
  and `SyncMode`, just doesn't call them on every
  append by default. rsx-dxs's 10 ms / `sync_all()`
  cadence is the contrast.
- **Design / Multi-writer** — corrected (see oracle
  findings below). OSS supports concurrent writers
  serialised by a `metadata.cq4t` write lock; rsx-dxs
  is single-writer-per-stream by construction.
- **Design / Cross-host** — single-host OSS,
  commercial replication.
- **Guarantees table** — 14 rows, fully symmetric,
  every Chronicle row populated honestly.
- **Published performance numbers** — quoted Chronicle's
  own README: 99% < 0.78 µs / 99.9% 1.2 µs for 40-byte
  IPC at 10 M events/min, ~5 M msg/s for 96-byte msgs
  on i7-4790.
- **rsx-dxs reference numbers** — cited the three WAL
  benches (`wal_bench`, `wal_fsync_bench`,
  `wal_random_read_bench`) with their p50 figures from
  `docs/benches.md`.
- **The honest summary** — Chronicle wins on steady-
  state IPC latency and random seek; rsx-dxs wins on
  cross-host out-of-the-box and bounded default
  durability.
- **Sources** — README, How_it_works.adoc, FAQ.adoc,
  async_mode.adoc, chronicle.software/queue, plus
  internal cross-refs to rsx-dxs benches and
  `docs/benches.md`.

No code, no Cargo.toml changes, no new bench harness.
`rsx-dxs/compare/README.md` summary table row was
re-read and left as-is — the row was already accurate
("mmapped files / TCP", "n/a (durable log)", "disk",
"sub-µs IPC", "Java").

## Oracle review

Asked codex for substance-only review on:
factual accuracy, fair framing, guarantees symmetry,
the "why we include it" framing, and the "why no bench"
reasoning. Codex flagged six substantive issues.

### Taken (all six)

1. **Chronicle Queue is NOT single-writer in OSS.** The
   first draft repeated a common misconception. Chronicle's
   own README: "supports concurrent writers and readers
   even across multiple JVMs on the same machine." OSS
   serialises concurrent appends via a write lock in
   `metadata.cq4t`. Enterprise async mode is a separate
   buffered-queue product, not "the way to get multi-
   writer." → Rewrote `Design / Multi-writer`, the lead
   paragraph, and the guarantees-table row.

2. **"Per-record fsync option: no API" was too strong.**
   `ExcerptCommon.sync()`,
   `ChronicleQueue.lastIndexMSynced()`, and
   Chronicle-Bytes `SyncMode` all exist. The honest
   framing is "no default automatic cadence" — manual
   sync APIs do exist. → Rewrote the durability
   paragraph; split the guarantees-table row into
   "Default per-append sync" and "Manual sync API".

3. **Kernel/syscall language was over-absolute.** "Never
   enter the kernel" / "no syscall on the read path"
   ignores setup, page faults, rollover. → Qualified
   with "steady state" everywhere it appears (IPC
   paragraph, published-numbers section, guarantees
   table).

4. **Lock mechanism overspecified.** Asserted "file
   lock on metadata.cq4t" — the public docs support
   "v5 moved the lock state to metadata.cq4t" but not
   the exact OS mechanism. → Softened.

5. **Wire-format was framed as purely self-describing.**
   Chronicle also has `BytesMarshallable`,
   `writeBytes`/`readBytes`, `FIELDLESS_BINARY`. →
   Rewrote `Design / Wire format` to call the format
   a spectrum.

6. **`directory-listing.cq4t` was version-mixed.** That
   sidecar existed in v4; v5 folded it into
   `metadata.cq4t`. → Rewrote with the v4→v5 history.

Also took oracle's suggestion to make the
"why no direct benchmark" section explicit that a Java
JMH bench would be a different harness measuring a
different thing (not just "infeasible").

Also softened "closest peer" → "useful comparator on
the same design axis" per oracle's note (we haven't
done a systematic survey of every mmap-IPC product).

### Skipped

None. All six findings were substantive and acted on.

## Verification done

- WebFetch on Chronicle's README confirmed the multi-
  writer claim ("concurrent writers and readers even
  across multiple JVMs on the same machine") that
  oracle flagged. Verified before changing the doc.
- WebFetch on `How_it_works.adoc` confirmed `.cq4`
  cycle-rotated files, multi-level index
  (index2index → secondary), and `directory-listing
  .cq4t` v4→v5 history.
- WebFetch on `FAQ.adoc` confirmed the page-cache
  durability framing and the "not on the critical
  path" language quoted in the doc.
- Read `rsx-dxs/benches/wal_bench.rs`,
  `wal_fsync_bench.rs`, `wal_random_read_bench.rs`
  to anchor the rsx-dxs numbers. Reviewed
  `docs/benches.md` for p50 values quoted.

## Open questions

- The `~700 µs / ~7 µs per record amortised` figure in
  the rsx-dxs reference table is my estimate from the
  pattern of `wal_append_fsync_batch_100` (100 appends
  + one fsync). Should be reconciled with the actual
  Criterion run if anyone has a current bench-baseline
  for it. Did not re-run benches — outside scope.
- "OSS does not bound the durability window" is a
  defensible statement but a thorough deployment doc
  for Chronicle could specify per-deployment cadence
  configuration. The doc deliberately stops short of
  characterising every possible Chronicle config.
- Chronicle Enterprise has more nuance (async mode,
  replication, encryption) than this doc represents.
  The compare entry is intentionally OSS-vs-rsx-dxs;
  enterprise is mentioned only where relevant.

## Files touched

- `rsx-dxs/compare/chronicle-queue.md` — full rewrite
  (72 → ~290 LOC).
- `.ship/22-PROTOCOL-COMPARES/CHRONICLE-REPORT.md` —
  this file.

`rsx-dxs/compare/README.md` reviewed, no edit needed.
No Cargo / source / bench code touched.
