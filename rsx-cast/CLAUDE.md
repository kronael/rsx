# CLAUDE.md — rsx-cast

This file is local to the `rsx-cast/` crate. It records the README +
docs conventions specific to this crate. Inherits everything from the
repo-root `../CLAUDE.md`.

## Runtime — NEVER add one

`rsx-cast` has zero runtime deps and must stay that way. `CastSender`,
`CastReceiver`, `WalWriter`, `WalReader`, `ReplicationService`, and
`ReplicationConsumer` are all synchronous. Do NOT add monoio, tokio, or any
async executor as a dep — not as optional, not behind a feature flag.

**Specifically: do NOT add `monoio::net::UdpSocket` to `CastReceiver`.**
The caller (gateway, marketdata) owns the socket. rsx-cast takes bytes
in and returns records out. There is no speedup to be had from
integrating a runtime here — the async wakeup belongs in the caller's
event loop, not in this library. This was explicitly decided and is
not up for reconsideration.

## README style — patterns from rtrb

`rtrb` (https://github.com/mgeier/rtrb) is the reference for how a
high-quality Rust crate README reads. Match its tone and structure
where applicable. Concrete principles:

1. **One-sentence elevator pitch as the second line.** rtrb: "A
   wait-free single-producer single-consumer (SPSC) ring buffer for
   Rust." No preamble. The reader knows what the crate is by line 2.

2. **README is for orientation; API details belong on docs.rs / in
   specs.** rtrb has no API examples — just a `[dependencies]` snippet
   and links. We have more (we're a protocol, not a data structure)
   but the principle holds: don't duplicate what docs.rs / specs cover.

3. **No marketing language.** "Wait-free" is technical; "blazingly
   fast" is marketing. We commit factually-checkable claims only.

4. **No badges.** rtrb has zero badges (no CI, no docs.rs version, no
   crates.io). Header stays clean. We follow this — internal crate
   anyway.

5. **Honest performance section.** rtrb says explicitly that benchmarks
   are "deeply flawed and to be taken with a grain of salt. You should
   make your own measurements." We can keep our concrete numbers
   table — but qualify it with "loopback microbenches; production p50
   differs; run cargo bench yourself."

6. **Cite alternatives generously.** rtrb lists ~25 alternative
   implementations in Rust + 11 in other languages, with a one-line
   note for each. Frames itself as "if you don't like this crate, no
   problem." We mirror this in `compare/niche.md`; surface a 5-link
   subset in README.

7. **Acknowledge lineage.** rtrb has an "Origin Story" crediting
   crossbeam. We have one too: casting descends from LBM → Aeron → MoldUDP64.
   Should be a "Lineage" or "Acknowledgments" section.

8. **MSRV is explicit.** rtrb states "minimum supported rustc version
   is X.Y.Z" + policy on bumps (minor version bump on MSRV bump). Add
   the same to our README.

9. **Sections are short.** Most rtrb sections are 1-3 paragraphs. If
   ours runs longer, it probably belongs in ARCHITECTURE.md or a spec.

10. **Standard dual MIT/Apache license block + Contribution section.**
    The Rust crate template. rtrb uses the exact wording; we copy.

11. **Breaking-changes pointer.** rtrb links to GitHub releases for the
    changelog. We have CHANGELOG.md; link it.

12. **No architecture diagram in README.** Defer to ARCHITECTURE.md.

## Where we justifiably diverge from rtrb — DO NOT regress

rtrb documents a 200-LOC well-understood data structure. We document
a novel protocol + transport that readers won't already understand.
**Do not cut explanatory content just to match rtrb's minimalism.**
Cut fluff (marketing prose, redundant sections); keep substance.

Keeper sections — DO NOT remove or shrink these chasing rtrb-style
brevity:

- **"Why this exists"** — readers need to know what gap casting fills
  vs. the alternatives. rtrb skips this because everyone knows what
  a ring buffer is. casting needs the framing.
- **"Wire format"** — we're a protocol. The 16-byte header layout is
  load-bearing. rtrb has none because in-process structs need no
  wire spec.
- **"Guarantees"** — we make actual delivery promises (FIFO per
  stream, durability via WAL). Reader must know what they're getting.
- **"When NOT to use this"** — misuse on a lossy WAN would fail
  catastrophically. rtrb's failure mode (`Full` / `Empty`) is
  self-explanatory; casting's isn't.
- **"Requirements and assumptions"** — trust model is load-bearing.
  rtrb has no trust model to declare.
- **Specific bench numbers** — keep. rtrb is shorter because ring-
  buffer perf is well-understood; ours isn't.
- **Quick-start examples** — keep. The minimal-snippet-only approach
  rtrb uses only works because the API is two methods (`push`,
  `pop`). casting's send loop is non-obvious.

What to actually cut (from the punch list below): redundant
verbosity, stale claims, broken examples. NOT entire sections.

## When updating this README

Run through this checklist:

- [ ] Numbers in "How fast" match the most recent `facts/cmp-vs-udp-overhead.md`
- [ ] "What it gives you" doesn't reference removed features (e.g. `StatusMessage` was removed in 87b223e — don't resurrect)
- [ ] Quick-start examples actually use every binding they construct (no dangling `let mut wal = ...` that's never referenced)
- [ ] "Guarantees" reflects what the code does TODAY, not what we'd like (e.g. FAULTED escalation is specced but not implemented — note it's pending until that lands)
- [ ] Cross-references point to files that exist (not stale `.ship/` paths or removed specs)
- [ ] License block matches `Cargo.toml` `license = "..."` field exactly
- [ ] No "rollout" as a heading (per parent CLAUDE.md)

## Outstanding punch list (as of 2026-05-24)

From the docs review at `.ship/24-replication-DOCS-REVIEW/REPORT.md` — taken
from the oracle critique. **Each item is a sharpen, not a cut.** Apply
when ready; do not regress the README's overall depth.

- **README:36-40** "What it gives you" → first bullet still mentions
  `StatusMessage` and flow-control window. Those were removed in
  `87b223e`. Update to reflect: heartbeat (idle-only since 100ms
  cadence), NAK, retransmits — no flow control.
- **README:1-8** front matter could lead with the differentiator
  ("retransmit source IS the WAL") rather than the category label
  ("Log-backed reliable UDP transport").
- **README:139-149** "Guarantees" — "every record is delivered in
  sequence ... or the sender is notified it can't" — partly false
  today; reorder_buf overflow silently advances. Add caveat or wait
  for v4 to land (`.ship/26-CMP-RELIABILITY-V4/SPEC.md`) and then
  the claim becomes true.
- **README:106-121** Quick-start sender example constructs `wal` but
  never references the binding. Either reference it or remove the
  construction.
- **README:168-174** "When NOT to use" → expand to include: O(N)
  cold-tier retransmit (23.5 ms @ 10K records), per-packet recv
  allocation, single producer per stream, no congestion control.
- **README:12-19** "How fast" table → add a "Bench / env" column;
  add a footnote noting p99 not yet measured + production
  cross-process is 1128 µs not the loopback 10 µs.
- **Missing**: MSRV section (per rtrb principle #8).
- **Missing**: Lineage / Acknowledgments section (per rtrb principle
  #7) — credit LBM, Aeron, MoldUDP64, the projects rtrb itself is
  embedded in the wider repo.
- **Missing**: Breaking Changes link to CHANGELOG.md (per rtrb
  principle #11).
- **Missing**: Short "Alternatives" subsection at the end pointing at
  `compare/niche.md` + 3-5 directly-relevant links (per rtrb
  principle #6).
- **README:85-104** Install section: clarify "internal crate, not on
  crates.io, pin commit when used as a git dep."

## Standalone — no parent-relative paths

`rsx-cast` is positioned as the open-source extractable artifact.
The crate's docs MUST stand alone — assume the reader has only this
crate, not the parent `rsx` workspace.

- **NO `../foo` paths in `README.md`, `ARCHITECTURE.md`, or this
  `CLAUDE.md`.** A reader who clones the eventual standalone repo
  must not see broken links.
- **NO references to sibling crates** (`rsx-messages`, `rsx-types`,
  `rsx-matching`, etc.) except as context: "part of the wider rsx
  exchange project" is fine; pointing at their source isn't.
- **Specs (`specs/2/4-cast.md`, `48-wal.md`, `10-replication.md`, etc.)**:
  copy locally into `rsx-cast/specs/` if the README references them,
  OR drop the reference and inline the substance. Authoritative
  copy still lives in the parent repo's `specs/2/` for the
  exchange-wide story; the crate-local copy is for the standalone
  view. Sync drift is acceptable if either copy is dated.
- **Project-level docs (`docs/benches.md`, `facts/syscall-latency.md`)**:
  if the README needs them, either copy into the crate (e.g. as
  `rsx-cast/BENCHES.md` or `rsx-cast/facts/`) or inline the key
  numbers and drop the link. Don't `../`-link out.
- **Cross-project references are fine as full GitHub URLs**:
  e.g. `https://github.com/kronael/rsx/...` is OK; `../specs/...`
  is not.
- **The README's "See also" can list parent-repo files by name and
  describe them**: "the rsx exchange's matching engine consumes
  these records (separate concern; not bundled here)" — but no
  `../`-style link.

When you find a `../` link in any crate-local doc, replace or
remove it as part of the same change set.

## Numbers source-of-truth

When a README number disagrees with measurement, the chain is:

1. `cargo bench` is authoritative.
2. `facts/cmp-vs-udp-overhead.md` records dated measured numbers.
3. README / ARCHITECTURE quote from facts; cite the bench name.

If README and ARCHITECTURE disagree (they currently do on fsync —
24 µs batch vs 651 µs single record), one of them is wrong or
underspecified. Both numbers can be true if labelled by bench
variant.
