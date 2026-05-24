---
status: shipped
---

# WEDGE.md — picked

**Status: DECIDED (2026-05-21).**

The wedge is **B + A**: exchange-in-a-box SDK on top of an
open-source, orthogonal-parts library. Core thesis:

> The interesting reusable pieces are the transport
> (`rsx-dxs` = WAL + casting + replication) and possibly the orderbook
> (`rsx-book` = slab + CompressionMap + matching). Open-source
> those. Sell / support the full exchange-in-a-box on top.

The architecture already supports this — `rsx-dxs` shipped in
v0.2.0 with **zero `rsx-types` production dependency**
(`cargo tree -p rsx-dxs --edges normal | grep rsx-` is empty).
Any project that wants log-backed reliable UDP transport with
a TCP cold-path replay can `cargo add rsx-dxs` and use it.

The original draft below remains for context. The decision
section that follows it (§"What B+A looks like") replaces it.

---

## Why this exists

Four parallel skeptical reviewers — three cared about
engineering, one cared about narrative. Engineering verdict
clustered "solid prototype, fund the team." Narrative verdict
clustered "no wedge, no GTM, pass on the product."

Quote (compressed):

> Watch — Engage on team, Pass on product. The artifact is too
> good to ignore and too unfocused to underwrite. Pick one
> wedge and ship it: either (a) exchange-in-a-box SDK with a
> paying design partner, or (b) a niche venue (prediction
> markets, RWA perps, regional) where Hyperliquid isn't
> already entrenched. A reference implementation without a
> wedge is a GitHub star magnet, not a company.

This document is the founder picking one. Or saying no, RSX
stays a reference implementation, and the goal is reputation,
not revenue.

## Three honest options

### Option A — open-source reference implementation
**Goal**: be the public, citeable, audit-able implementation
of a low-latency perps exchange in Rust. The "Linux kernel of
exchanges." Revenue from talks, consulting, hiring funnel,
and possibly a paid support tier in year 2+.

- *Why it could work*: nobody has done this. CME / NASDAQ /
  HFT shops keep their internals private. RSX could be
  the artifact engineering teams use to study the design.
  The spec corpus + the brutal honesty culture is a
  differentiator.
- *Why it might not*: GitHub stars don't pay rent. Reference
  implementations are usually staffed by donations or
  university research, not VC.
- *6-month signal*: 3 inbound talks at exchange / HFT
  conferences. 1 production team citing RSX in a postmortem
  or design doc. 5+ contributors who don't share an inbox
  with the founder.

### Option B — exchange-in-a-box SDK (paid design partner)
**Goal**: package casting + WAL + matching + risk + recorder as
a deployable stack with operator tooling. Sell to one
design partner this quarter — a regional venue, a perps
sleeve inside an existing CEX, an institutional desk that
wants their own venue, a prediction-market protocol moving
off AMM.

- *Why it could work*: there are operators with real
  money who want a "Hyperliquid in a box" but can't or
  won't run on Hyperliquid's chain. Especially: regional
  jurisdictions, RWA venues, prediction-market platforms.
  The ask is "give us your stack; we'll customise the
  product layer."
- *Why it might not*: enterprise sales is a different
  muscle than building. The first design partner needs
  founder attention for 6+ months. Wrong partner = sunk
  cost.
- *6-month signal*: one signed design-partner contract,
  even at low price ($50–250k), with clear deployment
  milestone in month 3.

### Option C — niche venue (run it yourself)
**Goal**: pick one underserved asset class or jurisdiction
where Hyperliquid hasn't won. Run an actual venue. The
stack is the moat *only after* you have liquidity.

Niches that look open:
- Prediction-market perps (Polymarket-shape)
- RWA perps (treasury-yield, commodity-basket)
- Regional / jurisdictional venue (Singapore, UAE, Brazil)
- Esoteric basis trades (perp on a fund NAV, basket index)

- *Why it could work*: "RSX runs the only perps venue on
  X-asset" is a moat. If X grows, RSX grows with it.
- *Why it might not*: needs a token, a market-maker,
  custody, regulatory perimeter, KYC, and *liquidity*.
  None of these are in the repo. The build is 3-5x what's
  shipped.
- *6-month signal*: regulatory letter accepted, MM
  agreement signed, $100k notional traded on the venue
  (real, not founder-funded).

## What this quarter's work could include

For Option A:
- Submit the casting design as a paper to a perf-engineering
  venue (USENIX ATC, Sigmetrics, OSR).
- Polish the BLOG.md from technical brag-doc to narrative
  ("What we learned writing an exchange from scratch in
  Rust").
- Rewrite README §1 with a 30-second "what you can do with
  this" example.
- Open repo to outside contributors with a clear `good
  first issue` queue.

For Option B:
- Define the SDK boundary explicitly: what does a deploy
  look like? Helm chart? `cargo install rsx-stack`?
  Docker compose? Single binary?
- Reach out to 5 prospects this month. Friends of friends,
  not cold. Ask: "would you pay $X to run RSX as your
  internal exchange?"
- Build a paid POC slot ($25–50k, 4 weeks).

For Option C:
- Pick a niche by month-end based on:
  (a) regulatory clarity,
  (b) honest 24-month TAM you'd be happy with,
  (c) one MM and one design-partner LP you already know.
- Write a one-page memo on why this niche, what's the
  first contract, what's the unit economics.
- Find a co-founder for the GTM half.

## What NOT to do

- Try all three.
- Build for "fundability" before knowing who you're selling
  to.
- Write a token paper before the wedge is picked.

## Recommendation

Read the three options cold. The one that makes you most
uncomfortable is probably the right one — discomfort is the
shape of the work you haven't done yet.

If the founder is one person, Option A or B. If two people
with a GTM half, Option B or C. Option A as a *secondary*
is fine alongside B or C; Option A as a *primary* needs a
separate funding model (consulting, sponsorship, university).

A decision (even "I refuse to decide; A by default") moves
the next conversation from "watch" to "engage." Indecision
keeps the project at watch.

## Out of scope for this doc

- Pricing model details (defer to BUSINESS.md once wedge is
  picked)
- Technical roadmap (defer to per-wedge plan)
- Hiring (defer)

---

## What B+A looks like (decided)

Picked: **B (exchange-in-a-box) on top of A (open-source
orthogonal parts)**.

### The orthogonal parts (open source, MIT/Apache-2)

These ship as standalone reusable libraries. Each has its own
crate, its own README, its own benchmark, and is provably
usable without the rest of RSX:

- **`rsx-dxs`** — the load-bearing one. Log-backed reliable
  UDP transport (casting) + TCP cold-path replay (replication). Wire =
  disk = stream. Zero heap on send path. Two-tier NAK
  retransmit (ring → WAL). V0/V1 schema version byte. Already
  proven domain-agnostic in v0.2.0. **This is the headline
  artifact.**
- **`rsx-book`** — slab-arena orderbook with `CompressionMap`
  price-to-index. 54 ns single fill, FIFO within price level.
  Generic over the order-id type — anyone building a matching
  engine can `cargo add rsx-book`.
- **`rsx-messages`** — the example domain layer on top of
  `rsx-dxs`. Demonstrates how to ship a wire schema. Other
  projects substitute their own.

The transport (`rsx-dxs`) is the wedge artifact. It has the
strongest "no one else has this" claim: nobody ships
log-backed reliable UDP in Rust with the WAL-as-retransmit-
source design. Aeron is JVM-only. kcp is C. QUIC has the
wrong shape. This is the citeable thing.

### The packaged product (proprietary or source-available)

On top of the open libs, `rsx-exchange` is the deployable
stack: matching, risk, gateway, marketdata, mark, recorder,
maker, playground, web UI, ops. **This is what gets sold.**

Three flavours, picked per design partner:
- **Self-host SDK** — they run it on their infra. Helm chart
  / Docker compose / single binary. Source-available so
  they can read it; commercial license for production.
- **Managed instance** — we run it for them (regional CEX
  desk, prediction-market platform, basis-trade venue).
- **Vendor mode** — they fork the wire format, run their
  domain records, we license the transport + tooling.

### What this unblocks NOW

The decision unblocks a sequence of editorial + GTM moves
that were all blocked on "what's the story?":

1. **One-pager** (next deliverable). Two paragraphs +
   architecture diagram + three bullets per persona
   (engineer, exchange-operator, investor). Lives at
   root, links here.
2. **BLOG.md narrative reframe** (T5.2). Currently a
   technical brag-doc. Now reframes as "log-backed
   reliable UDP for Rust + the exchange that proves it
   works at line rate." cmp.md becomes the lead post.
3. **rsx-dxs/README.md polish** — already in good shape;
   add a "When you should use this" section, a comparison
   table vs. Aeron / kcp / QUIC (already in spec, port to
   README), and a 30-second example.
4. **BUSINESS.md draft** — pricing tiers (community free,
   SDK paid, managed instance), license posture (MIT for
   libs, source-available for `rsx-exchange`), support
   tiers.
5. **First-design-partner outreach** — 5 prospects this
   month, friends of friends, ask: "would you pay $X to
   run RSX as your internal exchange?" Goal: one signed
   $25-50k POC by end of quarter.
6. **Crates.io plan** — `rsx-dxs`, `rsx-book`, `rsx-types`,
   `rsx-messages` publishable as v0.2.0 once docs are
   library-quality (the rsx-dxs/README.md from v0.2.0 is
   close).

### What this still blocks

- **Token / chain decision** — only relevant if Option C
  (niche venue) ever gets picked up later as a second
  product line. Not in scope for B+A.
- **Regulatory perimeter** — depends on which design
  partner signs first. Defer.
- **Hiring** — depends on whether the first POC is signed
  before $$ runs out.

### The signal that B+A is working (6-month horizon)

- 3+ external projects citing or depending on `rsx-dxs`
  (open-source proof)
- 1 signed design partner for `rsx-exchange` (paid proof)
- 1 published-in-public technical artifact (paper, talk,
  or blog post) with reach > 5k engineers

If all three are missing at month 6, the bet wasn't right
— revisit between A-only (reputation play, alt funding) or
C (run a venue ourselves).

— file end —
