---
status: reference
---

# Spec consolidation audit (read-only)

Scope: `specs/2/` (47 specs) + `specs/3/` (1 spec, unrelated new-phase
work, no overlap with phase-2). No files edited except this report.

## Verdict summary

46 of 47 phase-2 specs are legitimately separate concerns. One real
(small) content duplication found, one index-completeness gap found.
No renumbering proposed (per `specs/2/index.md` "Numbering" section —
gaps are historical and intentional, out of scope).

---

## 1. WalHeader restated in three places — `propose-only`

**Specs:** `specs/2/48-wal.md` (Record Format v1, L28–43) vs
`specs/2/10-replication.md` (§1 WAL Record Format, L32–68) vs
`specs/2/4-cast.md` (§2 Wire format, L71–116).

**Overlap:** The 16-byte `WalHeader` struct (`version, _pad0,
record_type, len, _pad1, crc32, _reserved[4]`) is defined three times,
almost verbatim, in three different specs:

- `10-replication.md` L37–46: literal Rust struct + version-byte
  semantics + reader error behavior. This is the fullest, most
  authoritative copy.
- `4-cast.md` L78–94: same struct, plus wire-specific material that
  is NOT duplicated elsewhere (payload-size ceiling from `len: u16`,
  MTU/fragmentation reasoning, `PACKET_BUF_SIZE`). This copy earns
  its place — a protocol spec needs the byte layout inline, and it
  adds unique content.
- `48-wal.md` L28–43 ("Record Format (v1)"): this is the weakest
  copy. The file *opens* with an explicit disclaimer — "the concrete
  WAL implementation ... is specified in replication.md ... This
  document describes the shared design principles" (L7–10) — and
  then immediately restates the same field list in prose two
  sections later anyway. It adds nothing `10-replication.md` doesn't
  already say more precisely (it even says "Concrete record layouts
  are defined in **replication.md**" in the same section, L43).

**Proposed action:** trim `specs/2/48-wal.md`'s "Record Format (v1)"
section down to a one-paragraph pointer at `10-replication.md` §1
("see there for the byte layout"), keeping only the WAL-unique
content that section currently has no reason to duplicate (scope
note: matching-engine-only, v1; version-policy prose can also
shrink to a pointer since `10-replication.md` L48–65 already states
it in more operational detail — RecordType handling, `WalReader`
error variants, etc.). `4-cast.md` needs no change — its copy is
scoped to wire concerns, not WAL-file concerns, and carries content
`10-replication.md` doesn't.

**Blast radius:** `48-wal.md` / `48-wal` is referenced from ~50
files across the repo (grep `specs/2/48\|48-wal`), but almost all are
either the target file's own cross-references or the
`.matching-book-refine/` worktree mirror. The load-bearing ones (not
mirror, not self):
`GUARANTEES.md`, `CLAUDE.md` (component spec table), `CHANGELOG.md`,
`.diary/20260524.md`, `docs/concepts/{01-casting,07-asymmetric-durability,08-backpressure-not-drop,11-glossary}.md`,
`rsx-cast/{ARCHITECTURE.md,CLAUDE.md,README.md,src/header.rs,src/wal.rs}`,
`rsx-matching/src/main.rs`, `specs/2/{1-architecture,4-cast,45-tiles}.md`,
`specs/index.md`, `specs/2/52-blog.md`, `blog/*.md`. None of these
quote the WalHeader field list itself — they reference `48-wal.md` as
"the WAL spec" for backpressure/flush/rotation behavior, which this
edit does not touch. Trimming the duplicated struct section is safe;
no link targets change (same filename, same anchor
`#record-format-v1` can be kept even if the body shrinks).

**Verdict:** `propose-only`. It's a real duplication and a clean,
low-risk trim, but it's editorial (three sentences vs one paragraph),
not a structural fragmentation problem — founder call on priority.

---

## 2. specs/2/index.md is missing 5 specs — not a consolidation
finding, but directly relevant to a "spec fragmentation" audit

`specs/2/index.md`'s master table omits five files that exist in
`specs/2/`: `51-cmp-v2-multicast.md`, `52-blog.md`,
`53-read-service.md`, `57-config-server.md`, `58-http3.md`. Three of
these (57, 58, and by inspection 51/52/53) DO appear in the root
`specs/index.md` phase-2 table, so the content isn't undiscoverable,
but `specs/2/index.md` — the file this audit was told to treat as
"the master table of all specs" — is stale relative to the directory
it indexes. This isn't fragmentation (each spec is still one file,
one concern) but it's the kind of drift that makes fragmentation
audits like this one harder to trust next time. Recommend (not
performed here, since edits were out of scope): add the 5 missing
rows to `specs/2/index.md`'s table next time someone touches that
file.

**Verdict:** `propose-only` (documentation hygiene, not a spec
merge).

---

## 3. Perf/telemetry/latency-observability cluster — `keep-separate`

**Specs:** `specs/2/22-perf-verification.md` (251 lines),
`specs/2/33-telemetry.md` (296 lines),
`specs/2/59-latency-observability.md` (96 lines).

This was the cluster specifically flagged for scrutiny. Read all
three in full. They do NOT redundantly cover "how latency is
measured" — they cover three different layers with an explicit
division of labor, and `59` states its own boundary against the
other two in its own References section (L89–96):

- **`22-perf-verification.md`** owns *offline/synthetic*
  measurement: Criterion microbenches (§1), the Criterion regression
  gate `scripts/bench-gate.sh` (§2), the Playground gateway-roundtrip
  HTTP endpoint and its Playwright tests (§3), the E2E probe harness
  that opens a WS and times a real fill (§4), and a taxonomy section
  (§6 "What the numbers mean") explicitly warning readers not to
  confuse microbench / design-budget / playground-roundtrip numbers.
  This is bench-and-gate-shaped content — CI/dev-loop verification,
  not a production telemetry pipeline.
- **`33-telemetry.md`** owns the *production metrics substrate*:
  structured JSON log lines → rsyslog → Vector → Prometheus (full
  pipeline diagram, L19–24 and L276–296), per-component metric field
  catalogs (§Metric Fields, L53–89), the Vector transform config
  (§Metrics Extraction, L147–213), and an explicit "what processes do
  NOT do" list (L250–258: no Prometheus client lib, no push SDK, no
  per-process `/metrics`). No latency-*design* content here — it's
  infrastructure plumbing.
- **`59-latency-observability.md`** is the newest (`status: planned`)
  and is written as connective tissue, not a third redundant
  treatment: its Problem section (L7–19) explicitly names two
  questions — per-event (not covered by 22 or 33) and aggregate
  (explicitly deferred to 33's pipeline, L60–69: "ride the telemetry
  path already specced in `33-telemetry.md`... Latency histograms...
  are added to the Vector transform in `33-telemetry.md`"). Its
  Design section adds genuinely new content neither other spec has:
  per-hop `*_ns` timestamp fields embedded in wire records
  (`gw_in_ns`, `risk_in_ns`, `me_in_ns`, `match_done_ns`,
  `gw_out_ns`), the clock-sync assumption (PTP in prod, single-box
  dev), and why the client `net` leg can never be a server timestamp.
  Its References section (L89–96) cites both 22 and 33 by exact role
  ("how latency is measured/gated" vs "the structured-log → Vector →
  Prometheus pipeline") — the cross-referencing is already correct
  and precise, not accidental.

No section in any of the three restates content owned by another.
`22`'s Playground/E2E-probe material and `33`'s Vector/Prometheus
material never describe the same mechanism twice; `59` is scoped
purely to per-hop timestamp *design* (a gap both other specs
explicitly don't cover — 22 measures externally via probes/benches,
33 has no latency-specific histograms defined yet, only the general
metric-field catalog).

**Blast radius (for completeness, no action needed):** `22` is
referenced from `BLOG.md`, `rsx-playground/pages.py`, `CLAUDE.md`,
`README.md`, `.diary/20260501.md`, `.ship/18-META-REVIEW/`,
`.ship/27-REFINE-AUDIT/`, `.ship/41-MATCHING-RELEASE/PLAN.md`,
`specs/2/4-cast.md`, `specs/2/45-tiles.md`, `specs/index.md`,
`specs/2/59-latency-observability.md`. `33` is referenced from
`BUGS.md`, `docs/concepts/11-glossary.md`, `specs/2/1-architecture.md`,
`specs/2/59-latency-observability.md`, `specs/index.md`. `59` is
referenced from `BUGS.md` and `specs/2/index.md` (new spec, few
inbound links yet — expected for `status: planned`).

**Verdict:** `keep-separate`. This is the cluster the founder
suspected might be fragmented, and it isn't — it's a rare example of
a spec set doing cross-referencing right (each new spec states which
existing spec it extends and where the new content ends and old
content resumes).

---

## 4. Dashboard specs (7 / 12 / 23 / 27 / 54 / 55) — `keep-separate`

Delegated read (full text) to a sub-agent for volume reasons; verified
the sub-agent's headline claim by spot-checking scope statements
(each of 7/12/23/27 opens with an explicit target-audience line: user
support, systems ops, dev/test control plane, risk ops respectively,
and 23/27 each contain an explicit "not in scope: see spec N" line
pointing at the other). `54-tui-access.md` is the access/transport
mechanism (SSH forced-command dispatch + deferred web-terminal); `55`
is the terminal's own UX (perps screen, order entry, multi-market
roadmap) — cross-referenced, not duplicated.

**Verdict:** `keep-separate`, all six. Four different audiences that
happen to all render as "a dashboard" is not fragmentation.

---

## 5. Wire/transport cluster beyond WalHeader (4/10/18/48/6/29/49) —
`keep-separate`

Beyond the WalHeader duplication in finding 1, no other overlap
found: `4-cast.md` (UDP transport + NAK loss recovery), `10-replication.md`
(TCP catch-up + replay server + consumer), `18-messages.md` (concrete
record type catalog — FillRecord, BBO, OrderUpdate, ...),
`6-consistency.md` (ME event fan-out ordering guarantees),
`29-rpc.md` (async RPC request/response framing), `49-webproto.md`
(external client WS/protobuf protocol) are six genuinely different
layers of the same stack (physical transport → replication →
record catalog → fan-out semantics → RPC → client-facing protocol),
each with its own concerns and no restated algorithms.

**Verdict:** `keep-separate`, all seven (counting `48-wal.md` net of
finding 1's trim).

---

## 6. Testing specs 34–44 + edge-case specs (24, 47) — `keep-separate`

`44-testing.md` is a pure router/overview (points at 34–43 and the
edge-case catalogs, no component detail of its own). Each of
34/36/37/38/39/40/41/42 pairs 1:1 with its named component spec
(the "Source" column in `specs/2/index.md`'s Test specs table makes
this explicit) — this is the deliberate pattern CLAUDE.md already
calls out as legitimate, not fragmentation. `43-testing-smrb.md` is
`reference`-status and sources from `notes/SMRB.md`, not a component
spec — different kind of testing doc (ring-buffer semantics), still
not overlapping any of 34–42.
`47-validation-edge-cases.md` (order-acceptance edge cases) and
`24-position-edge-cases.md` (position-tracking edge cases) cover
different state machines (an order being accepted vs. a position
already open) and don't restate each other.

**Verdict:** `keep-separate`, all 11 (34–44) plus 24 and 47.

---

## 7. Risk / liquidator / mark cluster (28 / 13 / 15) — `keep-separate`

`28-risk.md` mentions that liquidation and mark-price fallback exist
(a risk-engine consumer needs to know both are inputs to margin
math), but attributes each to its owning spec rather than restating
the algorithm — this is exactly the "audit-style 'X is unauthenticated'
finding is not automatically actionable" pattern CLAUDE.md already
warns against applying here: mentioning a boundary is not owning it.
`13-liquidator.md` and `15-mark.md` each own their respective
algorithms in full; no duplicated logic found.

**Verdict:** `keep-separate`, all three.

---

## Bottom line

- **1 propose-only edit**: trim the WalHeader restatement in
  `specs/2/48-wal.md` §"Record Format (v1)" down to a pointer at
  `specs/2/10-replication.md` §1. Low risk, ~50 references to
  `48-wal.md` exist but none quote the struct itself.
- **1 propose-only hygiene item**: `specs/2/index.md`'s master table
  is missing 5 rows (`51-cmp-v2-multicast`, `52-blog`,
  `53-read-service`, `57-config-server`, `58-http3`) that exist on
  disk and in the root `specs/index.md`.
- **0 consolidate-now**: no clear, low-risk merge was found anywhere
  in the 47-spec set.
- **45 specs confirmed keep-separate** across perf/telemetry,
  dashboards, wire/transport, testing, and risk/liquidator/mark
  clusters — the repo's "one spec per real concern" convention is,
  on this audit, actually being followed. The perf/telemetry cluster
  in particular (22/33/59), which was the specific trigger for this
  audit, turned out to be a good example of correct spec layering
  with precise cross-references rather than duplication.

No file under `specs/` was modified in the course of this audit.
