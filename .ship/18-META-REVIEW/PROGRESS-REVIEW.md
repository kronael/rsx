# 18-META-REVIEW — Progress Review and Course-Correction Proposal

Senior-engineer meta-review of the work since v0.2.0. Reads
the prior CTO+CEO reports, the 17-REFINE-2 arc, the
playground audit findings, the actual code at HEAD
(`daafdfb`), and the diff between the docs and reality.
Adversarial by design — `meta/cto-ceo-review.md` says
"balanced reviews don't catch the drift."

The review style follows the CTO+CEO template: Verdict /
Strengths / Risks / Forced-rank / Surprises /
Out-of-scope. The dual-lens methodology is the right one;
this review's job is to point out where the team strayed
from it and what to do next.

## 1. Verdict

**The project is drifting, not off the rails.** The
engineering culture is real — three nested dual-lens audits
(playground audit → CTO+CEO review → this meta-review) all
found bugs that were genuinely fixed. The team's "we audit
our own work" reflex is the load-bearing strength.

But the last 24 hours produced **78 commits in one
calendar day** (the count moved by 2 while this review was
being written — still climbing) with no validation step between sprints, and the
output shows the signature of an over-amped agent loop:
half-fixed acceptance criteria, duplicate commits with
mismatched messages, a sealed CI gate captured before the
bug it gates was fixed, a brand-new crate spawned for a
single shared logging primitive, and an unrelenting
documentation lag that now spans rsx-maker (deleted but
documented as shipped) and rsx-log (created but documented
nowhere). The CEO findings the project promised to action —
Trade UI dead, /orders never acks, dashboard formatter
gaps — are mostly half-fixed; the latency arc is mostly
over-shipped. The forced-rank in `.ship/16-CTO-CEO-REVIEW/
SYNTHESIS.md` was correct; **execution converged for the
first ~30 commits, then spiralled for the last ~25**.

The single most important course correction: **stop, deploy,
verify, document, then resume**. Right now the team is
writing code faster than it is checking that what it
already wrote works in production. The latency arc itself
is a symptom — it's the third measurement in two days, with
no independent verification, and each measurement
contradicts the prior one.

---

## 2. Top 7 strengths (don't break these)

1. **The dual-lens audit cadence works and surfaces real
   bugs.** F1 (CMP `AddrInUse` restart loop) and F22
   (latency probe matches any fill) are exactly the kind of
   credibility-destroying bugs that single-reviewer audits
   miss. `meta/cto-ceo-review.md` is the most important
   document in the repo right now. Keep running it.

2. **The team accepts "no" verdicts.** Both CTO and CEO
   said no to greenlight; the response was a plan to fix,
   not a defense (`.ship/17-REFINE-2/PLAN.md`). That's rare
   and worth protecting.

3. **The spec/code cross-reference discipline is real.**
   `specs/2/6-consistency.md:163-172` enumerates 7
   invariants with named enforcement sites; the comments at
   the code sites cite the invariant numbers
   (`rsx-matching/src/main.rs:504,533,570`). This is the
   project's main lever for catching drift.

4. **Trust-boundary discipline is codified and survived a
   stress test.** `bde3211 [revert] cmp: drop source-IP
   filter` is the kind of commit most teams would never
   ship — accepting that a "security finding" is wrong-layer
   instead of writing the wrong code (`CLAUDE.md` "Trust
   boundaries"). Don't lose this.

5. **The probe race fix at `82e9966` is the strongest
   single artifact in the round.** The diagnosis chain
   (broken pipe → per-stage trace → U-after-F race →
   FillRecord wire change) is real systems engineering, not
   guesswork. The fix isn't perfect (see R3) but the
   debugging arc is exemplary.

6. **rsx-dxs is genuinely reusable.** Zero rsx-types
   production dep, `CmpRecord` trait abstraction, WAL+CMP
   library. If anything from this codebase is worth carrying
   forward, it's this.

7. **Off-hot-path latency tracing (`08cb179`) is the right
   design pattern.** rtrb SPSC + drain thread is the
   textbook answer to "tracing on the hot path is too
   expensive." Even though the new crate is overengineered
   (R7), the primitive is reusable and the hot-path cost
   measurably dropped from ~20µs per emission to ~30ns.

---

## 3. Top 10 risks (forced rank, with severity)

### R1 — The "proven Rust GW→ME→GW p50 = 1.128ms" headline is not actually proven [critical]

`.ship/17-REFINE-2/FINAL-REPORT.md:184-189` claims "**Rust
GW→ME→GW p50 = 1.128 ms**" as "proven." The actual
measurement
(`.ship/17-REFINE-2/STAGE-LATENCIES.md:20-27`) is a
producer-side 6-stage trace joined by oid — not an
independent end-to-end observation. The sprint also shipped
a native Rust probe specifically designed for ground-truth
e2e measurement (`rsx-cli/src/bench_probe.rs`, commit
`59965c5`), but **its observed RTT is not the basis of the
final claim**. Codex (oracle pass for this review) confirmed
this is the strongest evidence against the framing: "the
one shipped independent native probe is not the basis of
the final claim."

Compounding: the off-hot-path measurement (`SPEED-OFFHOT.md`)
moved the total from 1163 µs to 1143 µs — a 20 µs delta
that proves tracing wasn't the bottleneck, but does NOT
validate the 1128 number as ground truth. The same
"emissions ≈ 4 µs each" estimate inside `SPEED-GRANULAR.md`
contradicts the earlier "~15 µs each" estimate; the report
itself admits the per-emission cost is unclear.

The Python e2e probe says 11878 µs p50. The Rust per-stage
sum says 1128 µs. Without an independent native probe
measurement, the ratio (1128 / 11878 = 9.5%) is an
inference, not a measurement.

**Acceptance test for fix**: run `rsx-cli bench-probe`
against a live cluster, capture its measured e2e p50, and
update the headline to whatever IT says. If it's also
~12 ms (because both probes share an aiohttp-class WS
overhead), then the 1.128 ms number is a per-stage
attribution, not a headline. If it's much lower (no Python
overhead), then 1.128 ms is closer to truth. Either way,
the current headline is unsupported by the data shipped.

### R2 — `bench-reference.json` is sealed on a known-broken probe [critical]

`bench-reference.json` was committed at 14:22 (commit
`5032085`, sealed `p50=11780us`). The probe race fix landed
at 17:48 (commit `82e9966`). **The CI regression gate is
gating against a number captured 3.5 hours before the bug
it depends on was discovered and fixed.** Any PR that
genuinely degrades latency by 9 % will still pass the gate
because the "floor" includes the probe race's noise. The
`_comment` field in `bench-reference.json` correctly notes
"current p50=11780us reflects the cluster as-measured
2026-05-22 — see .ship/17-REFINE-2", but does NOT note that
this measurement was taken with a broken probe.

**Acceptance test**: re-run `latency-publish.sh` after the
probe fix, re-seal `bench-reference.json` with N≥1000
samples, document the timestamp explicitly.

### R3 — FillRecord wire-format change without a version bump [critical]

`rsx-messages/src/lib.rs:69` adds `taker_ts_ns: u64` at
offset 88, where previously there was implicit padding. The
WAL header version was NOT bumped (`rsx-dxs/src/header.rs:43`
still defines `V1=1` as latest). The runtime mitigation is
a plausibility check at `rsx-gateway/src/route.rs:52` and
`rsx-risk/src/main.rs:660`: `if taker_ts_ns >
1_700_000_000_000_000_000 { use it } else { fall back to
ts_ns }`.

Codex (oracle pass) confirmed this is a real
wire-compatibility risk understated in the original review:
"`WalHeader.version` governs header framing, not payload
schema. The code now tells readers to interpret previously-
unused payload bytes semantically with only a plausibility
guard. This is one of your strongest points." Stale WAL
files from before this change have undefined memory in
those 8 bytes. Any byte pattern that happens to numerically
exceed 1.7e18 will be interpreted as a valid t0_ns anchor,
producing nonsense latency deltas on replay. Schema drift
without a version bump is exactly the bug the V0/V1 split
existed to prevent.

**Acceptance test**: bump `WAL_HEADER_VERSION_LATEST` to
`V2`; reject V1 FillRecords from being interpreted with the
`taker_ts_ns` field; emit a one-time WARN per stream on
version downgrade. Or: change the offset-88 field to a
sentinel-padded discriminator that's always written
0xDEADBEEF in old records.

### R4 — `order_prod` ring still silently drops orders on risk side [critical]

`rsx-risk/src/main.rs:539` — after the F1.1 "silent fill
drop" sweep, the order path STILL has:
```rust
if order_prod.push(order).is_err() {
    warn!("order_prod ring full — dropping order");
}
```
No counter, no power-of-2 throttle, no escalation. Compare
to `fill_prod` at line ~680 which now stall-loops, or
`bbo_prod` at line 607 which counts drops with a
power-of-two warn throttle. F1.1 was reported as "all peer
sites" but the order-ingestion peer site was missed.

The CTO report's R1 acceptance test was: "grep `let _ =
.*push(` across `rsx-risk/src` returns 0 hits outside test
code." That grep was the assertion; the actual behavior
(silent drop) was not asserted. The order-side site uses
`.is_err()` instead of `let _ =`, so it passed the lint but
fails the spirit of the fix.

**Acceptance test**: integration test that fills `order_prod`
with N+1 orders, asserts either (a) CMP receiver stalls
(no order drops), OR (b) drops are counted in a
`/x/health`-exposed metric.

### R5 — `MAX_EVENTS` overflow panics instead of stalling, contradicting spec invariant #4 [important]

`rsx-book/src/book.rs:102-108` — `Orderbook::emit` now
asserts on overflow:
```rust
assert!(
    (self.event_len as usize) < MAX_EVENTS,
    "INVARIANT: ME event buffer overflow ..."
);
```
`specs/2/6-consistency.md:168` says: "Matching engine never
drops events (ring full = stall)." Panic and stall are
different semantics — panic crashes the process and forces
WAL replay; stall holds the producer.

Codex (oracle pass) confirmed the spec/code mismatch but
added nuance: "this buffer is a per-order scratch array
with no concurrent consumer, so literal 'stall' is not
implementable here. The real issue is spec/implementation
mismatch, not just 'panic bad.'" If the team intends
fail-fast on this invariant, the spec should say so. If
the team intends stall, the code should stall. Today the
two contradict, and the comment at `book.rs:96-100` calls
the overflow "an unrecoverable bug" — which is a third
position that's compatible with neither.

**Acceptance test**: update `specs/2/6-consistency.md:168`
to "Matching engine never drops events; per-order event
buffer overflow is fail-fast (process aborts, WAL replay)."
OR change the code back to a producer-side stall.

### R6 — Documentation drift is now systemic, not local [important]

The CHANGELOG, CLAUDE, README, ARCHITECTURE, PROGRESS,
FEATURES, BLOG all reference **`rsx-maker`** as a shipped
crate. `rsx-maker` was deleted in commit `f7fce24` (the
deletion commit message explicitly says "dead code — playground
uses market_maker.py"). No document was updated. Citations:
- `CLAUDE.md:46` — "rsx-maker/ Market maker bot"
- `README.md:164` — "rsx-maker/ Market-maker bot"
- `README.md:312` — "rsx-maker uses blocking tungstenite"
- `ARCHITECTURE.md:86` — "rsx-maker/ Market-maker bot"
- `FEATURES.md:155` — "### rsx-maker"
- `PROGRESS.md:52` — "| rsx-maker | shipped | ..."

Conversely, `rsx-log` (added in commit `5ad7d91` as a NEW
workspace member) appears in ZERO documentation files.
The 12-crate count remains correct only by coincidence —
one crate was deleted, one was added, the docs drift in
both directions.

Other drift:
- README/PROGRESS/CHANGELOG say **878 tests passing**;
  FINAL-REPORT says 885; CTO-REPORT says 923; MEMORY says
  882. Five sources, four numbers.
- `<50 µs` GW→ME→GW still cited in `CLAUDE.md:24`,
  `CLAUDE.md:232`, `README.md:6`, `README.md:207`,
  `ARCHITECTURE.md:200`, `FEATURES.md:9`, `BLOG.md:91`,
  `CHANGELOG.md:99` — eight live citations. The CTO
  report's acceptance test said "strip the <50 µs language
  until the measurement matches the claim." Not done.
- `TODO.md:28` says "All 28 audit findings closed."
  FINAL-REPORT.md says "the live cluster is still running
  pre-refine binaries" because of a session-collision
  deployment gap. Both can be true only if "closed" means
  "fixed in code, not validated in production." That's not
  what "closed" means in any other software shop.

Codex flagged this severity should be escalated: "Missing
`.diary/`, stale counts, stale crate references, stale
budgets, and a pre-fix sealed `bench-reference.json`
together look like control loss, not ordinary doc lag." I
agree.

### R7 — `rsx-log` extraction is mildly over-engineered [important]

Commit `5ad7d91` extracted the off-hot-path tracing
primitive from `rsx-types` into its own crate (`rsx-log`).
The commit message justifies it as "rsx-types is the
foundation crate; pulling rtrb+tracing into it forces those
deps onto every downstream component including rsx-dxs
which is supposed to be domain-agnostic." But
`rsx-dxs/Cargo.toml` already depends on `tracing` directly;
the "domain-agnostic" claim is true only at the type level,
not the dep graph level. Codex pass: "Mildly overengineered,
not a major mistake."

The deeper concern is the **velocity at which the team
spent the innovation token**. Per CLAUDE.md's "Boring code
philosophy": "You get ~3 innovation tokens, spend on what
matters." Adding a new crate to host one shared SPSC-
buffered logging primitive used by 3 binaries (rsx-gateway,
rsx-matching, rsx-risk) is on the marginal side of the
cost/benefit ratio. The same primitive could live in
`rsx-types/src/log.rs` with a feature flag.

**Acceptance test**: if `rsx-log` is justified, then add a
test that proves: (a) the per-call cost is <100ns in
release builds, (b) the drain thread keeps up at 10k
orders/s, (c) the bounded ring overflows under stress and
logs the count. Today only (a) is informally claimed by
SPEED-OFFHOT.md.

### R8 — Hot path is now interleaved with telemetry comments and emission blocks [important]

`rsx-matching/src/main.rs:460-613` — the matching engine
main loop now contains 7 latency-sample emissions
(`me_in`, `me_dedup_done`, `me_wal_accepted_done`,
`me_match_done`, `me_wal_events_done`, `me_index_done`,
`me_out`), each wrapped in a 5-line block computing
`time_ns() - timestamp_ns`. Per CLAUDE.md "Code Style":
"NEVER add comments unless the behavior is shocking and not
apparent from code or logging." The latency-trace comments
are not shocking; they document a single design choice
(stage anchor) seven times.

The cognitive cost of reading the matching loop is now
dominated by telemetry, not by matching logic. This is the
**Heisenberg risk**: measurement code becomes part of the
codebase's identity, and removing it later becomes a
refactor instead of a cleanup. Codex agreed: "later spiral
into off-hot-path tracing ring extraction and extra
benchmark churn looks like scope drift while CEO-critical
issues stayed open by plan."

**Acceptance test**: refactor to a single per-stage helper
`stage(name, oid_hi, oid_lo, t0_ns)` that captures `now` and
calls `rsx_log::latency::sample` in one line. The 7 inline
blocks should collapse to 7 one-liners. If the goal is also
to make telemetry tunable, add a `RSX_TRACE_STAGES`
compile-time feature so production builds carry zero
overhead.

### R9 — Duplicate commits with mismatched commit messages [important]

`5032085 [fix] book: raise MAX_EVENTS to 65536` actually
touched `Makefile`, `bench-reference.json`,
`scripts/bench-gate-e2e.sh`, `specs/2/22-perf-verification.md`
— NOT `rsx-book/src/book.rs`. The commit message LIES about
its diff. The actual book change landed in `9159639` 90
seconds later. The REPORT.md at line 22 admits this and
says "Per repo rules (no destructive rewrite, never amend)
we kept both" — but kept both with mismatched messages.

This is a real artifact of the parallel-subagent pattern:
two agents holding overlapping files, both committed, one
ended up with the other's diff under its message. The repo
rule against amending is correct in principle; the **system
that produced two divergent commits with the same intended
content is a process bug**. The right repair is `git revert
5032085` and re-staging, which the team explicitly avoided.

**Acceptance test**: a CI hook that rejects commit messages
whose subject doesn't match at least one of the touched
file paths. The current "trust the agent" approach is
unsafe.

### R10 — Untracked WIP and post-FINAL-REPORT spree [nice-to-have]

While this review was being written, the team committed
`5583940 [bench] in-process e2e pipeline: real CMP/UDP,
real ME core` — a 602-line new binary
(`rsx-cli/src/bin/bench_e2e_pipeline.rs`) with **10 `let _ =`
violations** of the CLAUDE.md wisdom rule (lines 291, 326,
357, 364, 374, 465, 532, 595, 600, 601). Same commit set
also added the previously-untracked `rsx-dxs/benches/`
(cmp_one_way, cmp_rtt, udp_rtt, wal_fsync, wal_random_read).
**The wisdom-rule violation rate is increasing, not
decreasing.**

Also during the review window: `9bbb8f6 Revert "[perf] kill
the 100us sleep yields in gateway+marketdata CMP loops"` —
the yield_now change shipped 30 minutes earlier got
reverted. The revert commit has no body explaining why; one
plausible interpretation is "monoio doesn't actually
support a runtime-agnostic yield_now and the build broke."
**This is the second revert in the post-v0.2.0 sprint
(`bde3211` was the first, on the CMP source-IP filter).**
The first revert was disciplined and traceable. This one
isn't — no rationale in the body.

`rsx-webui/LOG.md` is also untracked — a scratch journal
that shouldn't be in version control but isn't gitignored.

**Acceptance test**: `git status --porcelain | grep -v
.diary | grep -v .ship` returns empty before any release
claim is made. Reverts must carry a one-paragraph rationale
that names what broke.

---

## 4. Forced rank: next 2 weeks to fix the project, not push features

Items 1-3 are "stop the drift before another sprint." Items
4-6 are "close the audit loop." Item 7 is "earn the right
to ship v0.3."

### Fix 1 — Validate, then declare. (Week 1, days 1-3)

The deployment gap noted in `FINAL-REPORT.md:91-105` is a
TOP fix priority. Until the refined binaries are running
on the dev cluster, the F1.1/F1.3/F2.2 acceptance tests
are unverified. Specifically:
- Restart the dev cluster on a port set the other session
  isn't holding (RSX_GW_WS_ADDR override).
- Run `make latency-publish` with N=2000 with the fixed
  probe.
- Run `rsx-cli bench-probe` (the native Rust probe)
  alongside, capture its number.
- Diff vs `bench-reference.json`; if the native probe
  measures meaningfully different from the Python probe,
  the headline number must reflect that.
- Update `bench-reference.json` to the new, post-fix value.

**Done when**: a commit message says "post-fix baseline:
e2e_us p50=X (Python), p50=Y (native Rust), per-stage Rust
p50=Z." Three numbers, one cluster, one run, one timestamp.

### Fix 2 — Doc reconciliation pass. (Week 1, days 3-5)

Every doc that references `rsx-maker` updates to remove or
to "removed in `f7fce24`." Every doc adds the `rsx-log`
crate. README/PROGRESS/ARCHITECTURE all show the SAME test
count, derived from one shared `make test 2>&1 | grep "test
result"` source. The `<50 µs` claim is either reduced to
"design budget; current measurement: X" or removed entirely
until X gets measured.

The CHANGELOG appendix for v0.3 names every claim that
changed during refine-2 (the latency number, the test
count, the rsx-maker delete, the rsx-log add, the
FillRecord wire field).

**Done when**: `grep -rn rsx-maker $(ls *.md)` returns
empty. `grep -rn "<50" *.md` returns either empty or
includes a measured value in the same line.

### Fix 3 — Close the half-fixes. (Week 1, days 5-7)

Three sites in the F1.1 family are still broken:
- `rsx-risk/src/main.rs:539` (order_prod silent drop)
- `rsx-cli/src/bin/bench_e2e_pipeline.rs` (8 `let _ =`
  violations in untracked work)
- `rsx-gateway/src/main.rs:365` (1 `let _ =` violation)

Plus the FillRecord taker_ts_ns wire change needs either
a `WAL_HEADER_VERSION_V2` bump OR a stronger sentinel
(e.g. `_pad1` byte set to 0xFF when `taker_ts_ns` is
populated). Plus the `MAX_EVENTS` panic-vs-stall spec
mismatch needs a one-line resolution in
`specs/2/6-consistency.md:168`.

**Done when**: `grep -rn 'let _ =' rsx-*/src/` returns
empty (including untracked work). Spec text matches code
behavior. WAL files written by V1 are unambiguously
distinguishable from V2.

### Fix 4 — Independent native probe is the headline. (Week 2, days 1-3)

Replace the Python aiohttp e2e probe as the headline
metric. Use `rsx-cli bench-probe` (with tokio-tungstenite,
no Python in the path) as the source of truth. Update
`scripts/latency-publish.sh` to use the native binary;
update `bench-baseline.json` schema to include both
numbers; document the Python overhead as the difference.

**Done when**: README "What's measured" table has two rows:
"GW→ME→GW e2e (Python WS aiohttp): X µs" and "GW→ME→GW e2e
(native Rust WS): Y µs."

### Fix 5 — One CEO-blocker per sprint, not "later sprint." (Week 2, days 3-5)

`.ship/16-CTO-CEO-REVIEW/SYNTHESIS.md:92-97` listed C3
(`/trade` Loading... forever) as out-of-scope for refine-2.
That was the right call AT THE TIME — refine-2 had 9 items
already. But the WEDGE.md proposition (exchange-in-a-box
SDK) is undermined every day the Trade UI is broken,
because the Trade UI is the first thing any prospective
integrator touches. Either: pick one of (Trade UI WS
reconnect, /orders ack feedback loop, dashboard
operator/user surface split) and ship it, OR explicitly
demote the WEDGE.md framing to "internal only" until those
surfaces work.

**Done when**: one CEO-flagged item from
`.ship/16-CTO-CEO-REVIEW/CEO-REPORT.md` §3 is in a
committed PR with a Playwright regression test.

### Fix 6 — Dual-lens audit with strict adversarial stance. (Week 2, day 5)

Run the dual-lens (CTO+CEO) audit per
`meta/cto-ceo-review.md` against `daafdfb`. The new round
should ask:
- CTO: "Have any claims in the FINAL-REPORT.md degraded?
  Specifically: is the 1.128 ms reproducible from a fresh
  cluster run? Does the F1.1 sweep actually cover all push
  sites? Did the doc drift get worse?"
- CEO: "Has the dashboard polling regression returned? Are
  the Trade UI / /orders / format issues fixed? Does the
  cluster start cleanly on a fresh machine via `playground
  start-all`?"

**Done when**: third-round CEO+CTO reports + SYNTHESIS.md
land in `.ship/19-CTO-CEO-REVIEW/`. Both must explicitly
read the prior reports and call out NEW findings vs
EXISTING findings.

### Fix 7 — Tag v0.3.0-rc1 only if 1-6 land green. (Week 2, day 6-7)

Today's TODO.md says "ready for v0.3 cut." That's wrong.
The refine-2 closeout shipped code that hasn't run on a
deployed cluster. v0.3.0-rc1 requires:
- Native probe + Python probe both measured (Fix 1)
- All docs consistent (Fix 2)
- All half-fixes closed (Fix 3)
- Independent native probe is headline (Fix 4)
- One CEO-blocker shipped (Fix 5)
- Third-round audit shows no regression (Fix 6)

If any of these aren't green, the rc1 tag waits.

---

## 5. Surprises (positive and negative)

### Positive

- **The team genuinely uses the dual-lens methodology.**
  Both CTO and CEO reports stayed in their lanes — CEO
  never read code, CTO never opened browser. The synthesis
  pass found cross-pollination items. That's hard to do and
  the team did it.
- **Codex pass on this review confirmed several of my
  judgments and ESCALATED a few I had soft-pedaled.**
  Specifically, codex flagged that the doc/process drift is
  "control loss, not ordinary doc lag" — stronger language
  than I would have used unprompted. Also flagged that I
  was overstating `rsx-log` as a "pure vanity innovation
  token" — it has a coherent architectural rationale even
  if the boundary is mildly over-drawn.
- **Trust boundary discipline survived a 24-hour sprint.**
  CMP is still unauthenticated, ME still doesn't validate,
  no source-IP filters got re-added. Easy to lose during a
  fast cycle; the team didn't.
- **The probe race fix arc is genuinely impressive
  engineering.** Diagnosing F-before-U through a
  six-stage trace, retro-buffering F frames keyed by oid,
  and pinning the misaligned anchor with a wire-format
  field — all in one session — is real. The fix isn't
  perfect (see R3) but the debug process is exemplary.

### Negative

- **The 9 plan items shipped, then 25 more commits
  happened.** Per `.ship/17-REFINE-2/REPORT.md:7-21`, the
  REPORT marks the plan complete at commit `ee30c37`. The
  next 25 commits — the FillRecord wire change, the
  jti propagation sweep, the rsx-maker delete, the
  off-hot-path tracing ring, the rsx-log crate, the
  yield_now fix, the gateway benches, the matching benches
  — were not in the plan. They came out of running the new
  binaries and discovering bugs. **That's diagnostic; in
  any other shop those 25 commits would be a new sprint
  with its own plan.**
- **The plan itself silently expanded.** `PLAN.md` line 87
  says "4 parallel general-purpose subagents (CLAUDE.md
  max)." The actual session also ran a CEO browser audit
  (subagent 5), a codex pass (subagent 6), and a maker
  diagnosis (subagent 7+). CLAUDE.md "Agents and Skills":
  "Spawn 1-2 subagents typically, NEVER more than 4." The
  rule was technically respected per-step but broken across
  the cumulative session.
- **MEMORY.md drift.** The CTO report flagged 18 `let _ =`
  violations; the workspace sweep claimed 19→0; the actual
  current count is 11 (plus 8 in untracked work). MEMORY's
  "Audit re-verified 2026-05-22: workspace src/ grep is now
  0 hits" is wrong as written.
- **Two duplicate commits with mismatched messages.**
  `5032085` and `9159639` are the visible failure of the
  parallel-subagent commit pattern. There may be more
  hidden under non-merge-race scenarios; codex flagged this
  as the part I'd UNDERSTATED about doc/process drift.
- **The latency arc spent ~17 of 76 commits on a single
  metric.** That's 22 %. Cumulative, including the
  `[bench]` commits that documented the work, it's higher.
  Meanwhile the Trade UI is still "Loading..." forever and
  the operator-vs-user UI separation is still TODO.
- **No `.diary/` entries anywhere.** CLAUDE.md "Documentation":
  "ALWAYS use `/diary` skill to write diary entries after
  significant work." 76 commits is significant work. Zero
  diary entries. Discoverability for the next reviewer is
  damaged.

### Expected but absent

- **A `rsx-cli bench-probe` output captured into
  `bench-baseline.json`**. The native probe was the explicit
  fix for "Python overhead masks the real number"; without
  its output, the claim is unverified.
- **A `LATENCY.md` or equivalent that summarizes the
  per-stage findings in one place outside `.ship/`.**
  `.ship/` is ephemeral by repo policy; the proven numbers
  will get pruned with the ship dir. They belong in a
  long-lived doc.
- **A `fuzz/` directory.** Still missing — CTO report flagged
  this in §5 (expected but absent); nothing shipped.
- **A `LOAD-TEST.md`.** Still missing — CTO report flagged
  this; nothing shipped.
- **proptest in Cargo.lock.** Still missing — for an
  exchange whose matching rules ARE the product, this is
  conspicuous.

---

## 6. Out-of-scope notes (cross-pollination)

Items I noticed that don't fit cleanly under one of the
above headings but should inform future passes.

- **The dashboard polling regression came back in a new
  form.** `aec96ec` added TTL caches on /x/health and peers;
  but new bench endpoints (added in untracked Cargo.toml
  changes) likely re-introduce uncached endpoints. Worth
  re-running the CEO's `curl -w '%{time_total}'` smoke
  matrix.
- **The "Deployment gap" section in REPORT.md is itself a
  policy gap.** The team's only validation pathway is
  "restart the cluster", but the cluster is shared across
  agent sessions. The right fix isn't "coordinate
  restarts" — it's **per-session ephemeral clusters on
  per-session port allocations.** A dev box should be able
  to host 3 concurrent clusters without collision. The
  playground tooling needs this.
- **The CTO report's R5 (CMP reorder-buffer overflow +
  NAK clamp) didn't appear in the 17-REFINE-2 plan or
  shipped fixes.** That's not necessarily wrong — the
  CTO acknowledged it as "deliberate per spec" — but the
  acceptance test ("drop 513 packets between ME and Risk,
  verify positions reconcile from WAL") wasn't added.
  Worth tracking as a v0.3.1 invariant gate.
- **The "session-history-aware" startup protocol in user
  CLAUDE.md was not followed.** No `.diary/` entries means
  no breadcrumbs for the next session. The user-level rule
  "Read project diary" produces empty results in this repo.
  The system needs either: (a) diary entries get written,
  or (b) the rule gets demoted.
- **The 5 untracked rsx-dxs benches (`cmp_one_way`,
  `cmp_rtt`, `udp_rtt`, `wal_fsync`, `wal_random_read`)
  haven't been reviewed by anyone except the agent that
  wrote them.** A `code-review` skill pass before they get
  committed would be cheap and high-value.
- **`scripts/tests/` is also untracked.** Looks like a
  Python test harness that wasn't checked in. Same fate
  as the rsx-webui/LOG.md scratch file. The team is
  accumulating dev artifacts in the working tree without
  a stash/commit/ignore policy.
- **The synthesis pass methodology section
  (`.ship/16-CTO-CEO-REVIEW/SYNTHESIS.md:177-196`) noted:
  "Worth scoping the CEO walkthrough more tightly (skip
  Docs sub-pages — they were noise)."** This meta-review's
  observation: the CEO walkthrough was actually too
  permissive — the CEO clicked into Control and rebooted
  the cluster (CEO-REPORT.md negative #6). Future runs
  should have a `--read-only` flag.
- **`rsx-webui` is essentially abandoned-ish.** The
  `LOG.md` is dated weeks ago. CEO finding C3 about Trade
  UI is its surface symptom. The team continues to ship
  Rust improvements while the WebUI rots. The WEDGE.md
  proposition rests on integrators using the WebUI; this
  is the second time it's been deprioritized.
- **The `Skills` section in CLAUDE.md user-level memory
  references `speccheck` but doesn't list `cto-ceo-review`
  as a registered skill.** Worth adding so it's discoverable
  by future sessions.

---

## Appendix A — Specific commits to undo, redo, or rethink

These are not "must-revert" — they're commits whose value
should be reconsidered before locking v0.3.0.

| Commit | Action | Rationale |
|--------|--------|-----------|
| `5032085` | Revert + re-commit with correct message | Commit message claims book change; actual change was bench infra. Mismatched message will mislead `git log` searches forever. |
| `bench-reference.json` seal | Re-capture | Sealed at 14:22, before probe race fix at 17:48. Gate is invalid. |
| `5ad7d91` (rsx-log extract) | Reconsider in v0.3.1 | Could be a single `rsx-types/src/log.rs` file gated by `feature = "latency-trace"`. Defer the dedicated crate until a 2nd consumer outside the workspace exists. |
| `2fc3bac` (FillRecord wire change) | Add a version bump | Either bump WAL_HEADER_VERSION_LATEST to V2, or change the offset-88 field encoding to be self-describing (sentinel byte). |
| `9159639` (MAX_EVENTS panic) | Spec update | Update `specs/2/6-consistency.md:168` to match the panic semantics, OR change the panic to a producer-side stall. |
| `f7fce24` (rsx-maker delete) | Doc reconciliation | The delete was correct; the doc updates that should have accompanied it never landed. Catch-up commit: "[docs] remove rsx-maker references; add rsx-log to crate inventory." |
| `daccaba` (per-stage tracing in matching) | Refactor to helper | The 7 inline emission blocks should collapse to 7 one-liners using a `stage()` helper. Reduces the matching loop's cognitive surface by ~80 lines. |

## Appendix B — Findings tally

| Category | Count |
|----------|------:|
| Critical | 4 (R1-R4) |
| Important | 6 (R5-R10) |
| Nice-to-have | (in surprises + appendices) |
| Positive strengths to protect | 7 |
| Forced-rank fixes | 7 |
| Doc-drift citations | 11 |
| Specific commits to rethink | 7 |
| Codex confirmations | 5 (codex agreed) |
| Codex pushbacks | 1 (rsx-log overstatement) |

Total actionable findings: **~50** (above the 40-item
target from the user prompt).

---

## Final 250-word summary

**Verdict: drifting.** The project is not off the rails;
the engineering culture and audit cadence are real and they
work. But the last 24-hour, 76-commit sprint shows the
signature of an over-amped agent loop: half-fixed
acceptance criteria (order_prod still silently drops, F1.1
is incomplete), a sealed CI gate captured before the bug
it gates was fixed (`bench-reference.json` p50=11780
captured 3.5 hours before the probe race fix at `82e9966`),
a wire-format change without a version bump (FillRecord
`taker_ts_ns` at offset 88, plausibility-guard
mitigation), 11 documents that still reference
`rsx-maker` (deleted in `f7fce24`), zero documents that
reference `rsx-log` (added in `5ad7d91`), eight live
citations of a `<50µs` design budget the team has now
measured to be 22× off, and a "proven 1.128ms" headline
that codex confirms is a per-stage attribution, not an
independent measurement. The Trade UI is still "Loading..."
forever. The CEO findings from `.ship/16-CTO-CEO-REVIEW/`
were 50% addressed; the team then shipped 25 more commits
chasing per-µs attribution while three CEO-blockers
remained open by plan.

**The single most important course correction**: stop
shipping until the deployment gap noted in
`FINAL-REPORT.md:91-105` is closed. Restart the cluster on
isolated ports, run BOTH the native Rust probe AND the
Python probe, capture both numbers into the same baseline,
update every doc to match, then resume. Code velocity
without validation velocity is moving away from v0.3, not
toward it.
