# 16-CTO-CEO-REVIEW — Synthesis

Both reports landed. CEO walked 16 UI tabs via `agent-browser`,
captured 25 screenshots, wrote `CEO-REPORT.md` (336 lines).
CTO read code + specs + ran `codex exec` on tricky sections,
wrote `CTO-REPORT.md` (604 lines). Each kept to its lens.

## Verdict (both)

**Both say NO to greenlight today.**

- **CEO**: "No, I would not greenlight this for fundraising
  or a customer demo today." Single biggest reason: dashboard
  self-thrashes (`/x/health` 75 s under load), Trade UI
  permanently "Loading...", raw i64 fixed-point rendered as
  USD values. *Engineering is plausibly real; the product is
  not assembled.*
- **CTO**: "No, I would not bet a customer SLA on this
  codebase today." Single biggest reason: load-bearing
  claims (<50 µs budget, zero-heap hot path, 10 invariants
  enforced, JtiTracker wired) do not survive contact with
  the source. *Engineering health is better than fundability
  would suggest. What's missing is a measurement-backed
  reality check on the headline claims.*

**Both arrive at the same conclusion through entirely
different evidence**, which is the strongest signal the
dual-lens design produces.

## Items both lenses flagged (TOP PRIORITY)

These appear in both reports — fix first.

### B1: Health truthfulness is half-fixed

- CEO R2: `/x/health` reports YELLOW=70 with header "GW:
  offline" AND `/verify` showing a FAIL row. The failing-
  invariant → red-health chain is not wired.
- CTO R6 (event drops) + R3 (jti dormant): the dashboard's
  health and security stories both have load-bearing claims
  that are structurally there but operationally inert.

**Action**: wire one failing invariant end-to-end into
`/x/health`. The CEO has the acceptance test
("/verify FAIL → /x/health RED").

### B2: Latency claim contradicts measurement

- CEO observed `/x/latency-regression` shows `--` while a
  measured `n=619` baseline exists (p99 = 233 ms = 4669×
  over budget).
- CTO R2: `bench-baseline.json` p50/p99 explicitly
  contradict the `<50 µs` headline still quoted in
  ARCHITECTURE.md, BLOG.md (already softened in `82f096d`),
  and parts of `specs/2/`. Spec is honest in one place;
  README/ARCHITECTURE are still claiming the budget.

**Action**: a single audit pass to find every place the
50 µs number appears and either qualify it as "design
budget" or remove it. Also wire `/x/latency-regression` to
the actual `e2e_us` block instead of `--`.

### B3: CMP counters are not real cluster numbers

- CEO R2 (3rd bullet): "Gateway→Risk 1117, Risk→ME 1117,
  ME→Mktdata 1117" — three different pipes, byte-identical
  numbers after 4 h of maker traffic (77k orders).
- CTO R4: CMP reorder-buffer overflow silently discards
  buffered packets at `cmp.rs:706-720`; NAK count clamp at
  `cmp.rs:281` makes cold-tier WAL unreachable for spans
  > 4096.

**Action**: F9 from the prior audit was *labeled* fixed but
the counter still ghosts. The CMP transport itself has two
silent-discard sites the CEO can't see but which fully
explain why the counters look implausible.

## Items only CEO flagged (UX / packaging / docs)

The CTO cannot see these from source alone. Treat as
product/UX work.

- **C1 (CEO R1, critical)**: raw i64 fixed-point rendered as
  USD on `/risk` (`COLLATERAL 999999972019150`), `/topology`
  (`bid=49900 ask=50100`), `/maker`, `/wal`. Every quant
  investor will read this as "founders haven't sat with an
  actual trader."
- **C2 (CEO R3)**: dashboard self-thrashes under polling
  load. `/x/health` 75 s, `/x/key-metrics` 15 s,
  `/x/pulse` 16 s. First paint of `/overview` is a wall of
  "loading..." panels.
- **C3 (CEO out-of-scope)**: `/trade` permanently shows
  "Loading..." and "connecting --". The trade UI is the
  surface a real customer would touch first.
- **C4 (CEO)**: orders on `/orders` stay at STATUS "sent"
  LATENCY "-" forever (no ack feedback loop in the UI).

## Items only CTO flagged (engineering)

The CEO cannot see these from the browser alone. Engineering
work.

- **E1 (CTO R1, critical)**: silent fill drop on risk ingest.
  `rsx-risk/src/main.rs:601` `let _ = fill_prod.push(...)`,
  5+ sites total. Directly violates invariant #4 "Position =
  sum of fills". This is the single most dangerous bug —
  any backpressure spike silently corrupts user balances.
- **E2 (CTO R3, critical)**: JtiTracker null-defeated.
  `rsx-auth/src/rsx_auth/jwt_util.py:14-23` emits no `jti`
  claim; `rsx-gateway/src/jwt.rs:107-110` accepts missing
  jti and returns `true`. The replay defence we just shipped
  in `72bd481` is inert for every token currently issued.
- **E3 (CTO R6)**: matching engine drops events when
  `event_len >= MAX_EVENTS=10_000`
  (`rsx-book/src/book.rs:88-94`), contradicting spec key
  invariant "ME never drops events". Was eprintln, now
  tracing::warn (`bbb0f9f`) — but the underlying drop is
  still there.
- **E4 (CTO out-of-scope)**: "zero-heap hot path" claim
  falsified by codex on dedup + order_index allocations.
- **E5 (CTO out-of-scope)**: 18 `let _ =` violations remain
  workspace-wide despite MEMORY.md's "audit complete" claim
  (memory is stale).

## Items they disagree on

None observed. Both agreed on the verdict and the broad
shape of the problems. Where they differ is *which side of
the same problem* they see — CEO sees the lying counters,
CTO sees the dropped events that produce them.

## Cross-pollination findings

Items each agent noted as "out of scope" that the OTHER
report's revision should pick up:

- CEO out-of-scope → CTO finding: "I noticed the
  ARCHITECTURE diagram in the walkthrough page shows `Risk
  → Postgres (write-behind)` but no other tab surfaces the
  PG write-behind latency. *That's the suspected cause of
  the 233 ms p99 tail.*" → CTO already has this on the
  refine list as "instrument PG write-behind."
- CTO out-of-scope → CEO finding: "I never opened the UI but
  I'd predict the `/x/key-metrics` Msgs/sec computation
  (F26's sliding-window fix) needs the maker quoting at
  visible rate to validate." → CEO confirms: dashboard
  shows 0 msgs/sec while maker placed 77k orders.

## Forced-rank refine list (input to .ship/17-REFINE-2/)

Synthesizing the union of both "top 3 fix" sections:

1. **Silent fill drop on risk** (CTO Fix 1 / matches CEO B3).
   `let _ = fill_prod.push(...)` → stall + log. Done when
   `grep -rn 'let _ = .*push(' rsx-risk/src` returns 0 outside
   tests.
2. **JTI emit from rsx-auth + reject missing jti at gateway**
   (CTO Fix 2). Done when integration test proves replay
   rejection on real tokens.
3. **Format every i64 through a tick-size-aware formatter
   in the playground** (CEO Fix 1). Done when `/risk`
   collateral shows "$1,000,000.00" and `/topology` bbo shows
   "0.0499 / 0.0501 / spread 0.0002".
4. **Polling thundering herd** (CEO Fix 2). Done when
   `/overview` paints in ≤ 500 ms, `curl /x/health`
   returns in < 200 ms.
5. **Wire failing-invariant → red health end-to-end**
   (CEO Fix 3 ≈ CTO B1). Done when killing `recorder` plus
   any verify FAIL drops health below 50.
6. **Land an honest release gate on `e2e_us`** (CTO Fix 3).
   Done when CI fails if p50 regresses > 10% from baseline.

Items 1, 2, 5 are correctness; 3, 4 are CEO-readability;
6 is process. The natural sprint shape: take 1+2+5 as a
"hard" track and 3+4 as a "soft" track, in parallel.

## Methodology score (self-assessment of the review process)

What worked:
- Strict tool boundaries (CEO=browser only, CTO=code only)
  produced cleanly disjoint findings with one zero-overlap
  flag.
- Both reached the same conclusion through different
  evidence — strongest possible signal.
- File:line and UI-path citations make every finding
  triage-able in under 30 seconds.

What to fix next time:
- The CEO agent spent ~22 min in agent-browser; the CTO
  ~14 min in codex + reads. Worth scoping the CEO walkthrough
  more tightly (skip Docs sub-pages — they were noise).
- Both agents independently noticed the latency problem
  from different angles; the methodology should ask them
  to explicitly NOT name an item the other lens would more
  naturally name, to force lens discipline.
