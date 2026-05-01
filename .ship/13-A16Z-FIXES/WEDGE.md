# WEDGE.md — pick one

Status: **draft for founder review.** Not auto-merged into
public docs. Read, mark up, kill or promote.

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
**Goal**: package CMP + WAL + matching + risk + recorder as
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
- Submit the CMP design as a paper to a perf-engineering
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

— file end —
