# DXS docs review

Reviewer: agent-a106a180aa32ea70d, 2026-05-24

Files reviewed:
- `rsx-dxs/README.md`
- `rsx-dxs/ARCHITECTURE.md`

Competitor docs read: Aeron (README + Best Practices + FAQ + Performance Testing wiki), Chronicle Queue (README + FAQ.adoc), Quinn (README), NATS JetStream (concepts page), MoldUDP64 (via secondary sources — the Nasdaq PDF was unreadable from WebFetch).

Oracle artifacts: `oracle-prompt.txt`, `oracle-output.txt` (this dir).

## TL;DR

1. **README and ARCHITECTURE disagree on the WAL fsync cost.** README: `651 µs (amortised over 10 ms batch)`. ARCHITECTURE: `24 µs`. Both real, different bench variants (single-record vs 64KB batch), but units unlabelled. ARCHITECTURE also still says `<10 µs (target)` for a number README already shows measured at `10.3 µs`. One source of truth needed.
2. **README "When NOT to use" is too short.** 4 bullets vs Chronicle Queue's comprehensive list. Missing: O(N) cold-tier retransmit (23.5 ms @ 10K records), per-packet recv allocation, single producer per stream, no congestion control, trusted-L3 dependency. The honest list is already in `specs/2/4-cmp.md` but never surfaced to README.
3. **Trust model is buried.** ARCHITECTURE has a clean "Trust model" section. README only mentions "no TLS" parenthetically inside "When NOT to use". An internal-only unauthenticated transport should declare this in paragraph 1.
4. **All performance numbers are p50-only and loopback-only.** Aeron publishes p99, Chronicle publishes p99.9. Showing p50 only makes our docs look curated.
5. **README front matter leads with a category label, not a differentiator.** Chronicle leads with "broker-less, off-heap, millions/sec." Ours says "log-backed reliable UDP transport." The real differentiator ("wire = disk = stream", "embedded retransmit horizon") is in line 7, not line 3.

## Per-competitor comparison

### 1. Aeron

| Aspect | Aeron | rsx-dxs | Gap / Action |
|---|---|---|---|
| First-30-sec hook | "Efficient reliable UDP unicast, UDP multicast, and IPC message transport" — generic, no number | "Log-backed reliable UDP transport (CMP) + TCP cold-path replay (DXS) for fixed-format binary records." — also factual, no number | Both dry. Ours is sharper IF "wire = disk = stream" is promoted to first sentence. |
| Performance numbers | None in README; deferred to separate benchmarks repo | p50 table with 6 measurements at the top | **We win.** Foreground this. |
| Guarantees stated | "high-throughput and low-latency" (no formal guarantees) | "wire = disk = stream" + dedup/idempotent replay (ARCHITECTURE only) | **We have specific guarantees; state them in README.** |
| Limits / when-not-to-use | None in README; FAQ scatters limits (term length, message size 1/8th term, MTU, JDK) | 4-bullet "When NOT to use" list | Aeron defers; we have it inline. **We're better positioned but list too short.** |
| Architecture diagram | None in README | ASCII in ARCHITECTURE only | Tie. |
| Tooling/ops | References Archive, monitoring, debugging — all on wiki | No ops section in README | Action: add a `rsx-cli wal dump` callout to README. |
| Install / quick start | Not in README; deferred to wiki | Cargo dep + 2 quick-start snippets | **We win**, but install lacks commit pin. |

### 2. Chronicle Queue

| Aspect | Chronicle Queue | rsx-dxs | Gap / Action |
|---|---|---|---|
| First-30-sec hook | "broker-less, off-heap Java library for ultra-low-latency, persisted messaging at millions of events/sec" — concrete differentiator + scale | "Log-backed reliable UDP transport ... for fixed-format binary records" — accurate but flat | **Lead with the differentiator.** |
| Performance numbers | 0.78 µs p99, 1.2 µs p99.9 same-machine; 20-176 µs p99 cross-machine; 5M msg/s | p50 only, ~10.3 µs RTT, no throughput | **Show p99 or admit not measured.** |
| Guarantees stated | "total ordering"; "no data lost"; "persists across restarts" | Implicit via "WAL = source of truth" | Explicit guarantee list would help. |
| Limits / when-not-to-use | Explicit list: off-heap caching → Map; high-freq updates → immutable; NFS unsupported | 4 bullets only | **Mirror Chronicle's completeness.** |
| Tooling/ops | Strong: `DumpMain`, `ChronicleReaderMain`, disk monitoring, `StoreFileListener`, pretoucher | `rsx-cli wal dump` exists but README doesn't mention it | **Add `## Tooling` section.** |

### 3. Quinn (QUIC)

| Aspect | Quinn | rsx-dxs | Gap |
|---|---|---|---|
| First-30-sec hook | Logo + badges + "pure-Rust, async-compatible" + "30+ releases since 2018" | Text-only | Fine — we're internal. Don't add badges. |
| Performance numbers | None in README | 6 measurements | We win. |
| Limits / when-not-to-use | None | 4 bullets | We win. |
| Install / quick start | `cargo run --example server/client` — runnable in 30s | Code snippets + reference to `examples/cmp_smoke.rs` | Quinn's example is more inviting. **Promote `cargo run --example cmp_smoke` to top-level.** |

### 4. NATS JetStream

| Aspect | JetStream | rsx-dxs | Gap |
|---|---|---|---|
| First-30-sec hook | "Built-in persistence system" — definition, no quantification | Definition + first number is `31 ns` append | We win. |
| Guarantees stated | **Explicit**: at-least-once, exactly-once, RAFT consensus, "immediate consistency", linearizable | Implicit | **Adopt JetStream's explicit consistency vocabulary.** |
| Limits | Embedded in retention policy section | 4 bullets prominent but incomplete | Ours more prominent, less complete. |

The lesson is **vocabulary discipline**: "at-least-once", "exactly-once", "linearizable" are terms readers already know.

### 5. MoldUDP64

The Nasdaq PDF was not WebFetch-readable. From secondary sources:

| Aspect | MoldUDP64 | rsx-dxs | Gap |
|---|---|---|---|
| Performance numbers | Not in spec | Yes | We win. |
| Guarantees stated | Gap-fill via separate retransmit server; sequence per session | Embedded retransmit | (See `rsx-dxs/compare/moldudp64.md` for the full comparison.) |
| Limits | Multicast-only, no flow control, no auth | Unicast, NAK flow control, no auth | Different model entirely. |

## Synthesis

### What competitors do better — 5 items with concrete fixes

1. **Chronicle leads with a sharp differentiator + scale claim in one sentence.** Our opener is a categorical label. *Fix:* rewrite README:1-8 to lead with "Reliable UDP whose retransmit source IS the WAL the producer writes for audit and replay."
2. **Chronicle and Aeron publish p99 / p99.9.** We show only p50. *Fix:* add p99 column to "How fast" or add footnote "p99 not yet measured."
3. **Chronicle's "When NOT to use" is complete and specific.** *Fix:* rewrite README:137-142.
4. **Chronicle's tooling section gives ops a checklist.** *Fix:* add `## Tooling` section listing `rsx-cli wal dump`, env vars, log-metrics format.
5. **NATS uses standard consistency vocabulary.** *Fix:* in README near wire-format, add one sentence: "Delivery: at-least-once over CMP/UDP, deduped at consumer via seq + tips. Replay over DXS/TCP is deterministic and resumes from `tip + 1`."

### What we do better but underplay — 5 items

1. **We publish actual measured numbers in README front matter.** Aeron, Quinn, NATS — none do.
2. **We have a domain-agnostic transport with zero workspace deps.** Real differentiator vs Aeron-Archive (separate JVM sidecar) and Chronicle-Network (separate library). ARCHITECTURE calls this out (lines 10-26); README never does.
3. **Our header is 16 bytes vs Aeron's 32, KCP's 24.** In `compare/*.md` but not surfaced in README.
4. **Explicit version-byte management.** V0 legacy, V1 current, additive record types.
5. **We document the trust model honestly.** Aeron is silent on auth; Quinn mandates TLS without discussing when TLS is wasteful.

### What we overclaim / mislead

1. **"Two-tier retransmit" is a packaging difference vs Aeron, not a protocol invention.** README should say "embedded WAL, not a sidecar" rather than implying a novel reliability primitive.
2. **README "How fast" hides that p50 loopback is NOT what production sees.** `MEMORY.md` records cross-process p50 = 1 128 µs because of the monoio 100µs sleep bug.
3. **ARCHITECTURE.md "Measured performance" `UDP round-trip <10 µs (target)`** — target met; README has measured `10.3 µs`. Replace target with measured.
4. **`WAL flush + fsync (64 KB) | 24 µs` (ARCHITECTURE) vs `651 µs` (README).** Both real; they measure different things. Unit labels needed.
5. **`MIT OR Apache-2.0` license declaration** is misleading for an internal-only crate that CLAUDE.md forbids publishing.

## Oracle critique

Summary verdicts (full output at `oracle-output.txt`):

- **#1 [CRITICAL] Performance story internally inconsistent, ARCHITECTURE stale.** TAKE.
- **#2 [CRITICAL] `specs/2/4-cmp.md:364-365` and `:384-386` say retransmit is "in-memory ring only" — contradicts the shipped two-tier design.** TAKE. **Outside our scope** — flag for separate spec-fix.
- **#3 [CRITICAL] README hides operationally important limits the spec already admits.** TAKE.
- **#4-#12 [SUBSTANTIVE]** All TAKE (front matter, wire=disk=stream framing, environment column, p99, trust model, completeness of "when not to use", Aeron framing, install, quick start).
- **#13-#16 [NIT]** All TAKE but cosmetic.

All 3 CRITICAL + 9 SUBSTANTIVE are worth acting on. NITs optional.

## Recommended edits

### CRITICAL

**§1. `rsx-dxs/ARCHITECTURE.md` — replace stale "Measured performance" table.** Targets-vs-measured cleanup. Both fsync numbers labelled by bench variant.

**§2. `rsx-dxs/README.md` — expand "When NOT to use this" to ~8 bullets** including: O(N) cold-tier retransmit, per-packet recv allocation, single producer/multicast absent, no congestion control, slow-consumer behaviour.

**§3. `rsx-dxs/README.md` lines 1-8 — rewrite front matter.** Lead with "Reliable UDP whose retransmit source IS the WAL." Trust model in front matter.

### SUBSTANTIVE

**§4.** "How fast" table — add "Bench / env" column. Footnote about p99 + production cross-process.

**§5.** Rewrite "Why this exists" to focus on "bytes never reformatted" — the actual saving.

**§6.** Frame "two-tier retransmit" as embedded vs sidecar (vs Aeron Archive), not as a new primitive.

**§7.** Install section — note internal-only, no crates.io, pin commit when used externally.

**§8.** Quick-start: fix the dangling `wal` binding in sender example. Explain why `tick()` / `recv_control()` are mandatory.

### NIT

**§9.** "99% sendto syscall" → "99 % kernel UDP send path."

**§10.** Add "Internal-use crate" disclaimer above license block.

## Open questions

1. **`.ship/23-CMP-RELIABILITY-FIXES/SPEC.md` does not exist** in this worktree (stale base). Resolved on master.
2. **`rsx-dxs/compare/moldudp64.md` does not exist** in this worktree (stale base). Resolved on master.
3. **`facts/closed-source-messaging.md` does not exist** in this worktree (stale base). Resolved on master.
4. **Trust model link choice.** Spec is canonical; ARCHITECTURE is more readable.
5. **`specs/2/4-cmp.md` retransmit-source contradiction is real** but outside this task's scope.
6. **p99 numbers are not in any current bench output.** Requires Criterion percentile output or custom harness.
7. **License question.** CLAUDE.md says "Do NOT publish externally" but README declares MIT/Apache. Clarify.
