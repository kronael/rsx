# CTO + CEO Adversarial Review — Methodology

A repeatable process for getting two distinct lenses on the
project: a CTO lens on the code/architecture, and a CEO lens
on the live product (playground UI). Inspired by VC dual due
diligence — engineering DD and commercial DD run in parallel,
reports compared.

This document records HOW we run these reviews. The actual
outputs live in `.ship/NN-CTO-CEO-REVIEW/` per session.

## Why two lenses, not one

A single reviewer collapses two distinct failure modes:
- "Beautiful code, but no one would pay for it" (CTO happy,
  CEO unhappy)
- "Compelling demo, but the code is a debt bomb" (CEO happy,
  CTO unhappy)

We need both signals separately. A single reviewer biases
toward whichever lens they prefer.

## Personas

### CEO lens (commercial DD)

- **Persona**: a founder or growth-stage CEO evaluating
  whether to greenlight the product for fundraising, customer
  demos, or a public launch.
- **Cares about**: positioning, demo flow, narrative
  coherence, "would I show this to a customer in 5 minutes",
  fundraising readiness, market fit, wedge clarity,
  competitive differentiation, trust signals.
- **Does NOT care about**: code style, test coverage,
  internal architecture choices unless they leak into the UI.
- **Tool**: live browser via `agent-browser` against the
  running playground at http://localhost:49171/. Reads UI,
  not source.
- **Output**: an actionable report listing what would
  win/lose a meeting with: a serious customer, a serious
  investor, a credible technical partner.

### CTO lens (engineering DD)

- **Persona**: a hired CTO walking into the codebase week
  one, evaluating engineering health, hiring needs, and
  technical risk.
- **Cares about**: architectural soundness, spec/code drift,
  invariant enforcement, scalability ceiling, security
  posture, test depth, deployment risk, "would I bet a
  customer SLA on this".
- **Does NOT care about**: marketing copy, UI polish,
  branding.
- **Tool**: `codex exec` (oracle) + direct file reads. Reads
  source, specs, tests, commit graph, NEVER the UI.
- **Output**: an actionable report listing top engineering
  risks, top strengths to protect, and a forced-rank of what
  to fix first.

## Scope boundaries (critical)

- **CEO MUST NOT** read source code, specs/, ARCHITECTURE.md.
  If the answer to "what is this?" requires reading source,
  that's a CEO finding ("the UI does not explain itself").
- **CTO MUST NOT** open a browser or run the playground. If
  the answer requires running the system, that's a CTO
  finding ("the code's correctness depends on a UI we can't
  audit from source alone").
- Both agents must report what they expected to find vs what
  they found — gaps are findings.

## Report structure (both lenses)

Each report follows this skeleton, in this order:

```markdown
# {CEO,CTO} Review — RSX ({date})

## 1. Verdict
One paragraph. "Would I {fund / hire / ship} this today?
Y/N + the single biggest reason."

## 2. Top 5 strengths (don't break these)
Numbered, with concrete citations (UI path or file:line).

## 3. Top 5 risks (forced rank)
Numbered, with concrete citations and severity (critical /
important / nice-to-have).

## 4. Forced rank: if I could fix only 3 things this week
Numbered. Each item must have a clear acceptance test ("done
when X").

## 5. Surprises (positive and negative)
Bullet list. What did you expect to find that wasn't there.
What did you find that you didn't expect.

## 6. Out-of-scope notes
Anything the reviewer noticed but explicitly ruled out of
their lens (CEO finding code smells; CTO seeing UI gaps).
These cross-pollinate to the other report's revision.
```

## Adversarial stance

Both reviewers are explicitly adversarial — they are paid to
find what's wrong, not to be balanced. A "balanced" review
is a polite review, and polite reviews don't surface the
things that need to be fixed. The "Strengths" section exists
only to protect things from accidental degradation during a
refine pass, not to soften the criticism.

## Cross-pollination

After both reports land, a synthesis pass compares them:
- Items both flagged → top-priority refine items.
- Items only CEO flagged → likely UX/positioning/docs work.
- Items only CTO flagged → likely engineering work.
- Items they disagree on → escalate to founder.

## Cadence

This review is run every time we're about to ship a public
milestone (v0.3.0, demo day, fundraising round, first paid
customer). NOT every refine pass.

## Skills + tools

- CEO: `agent-browser` CLI (installed at ~/.bun/bin/agent-browser).
  Skill: `browse` (see ~/.claude/skills/browse).
- CTO: `codex` CLI (installed; ChatGPT-OAuth on host ~/.codex).
  Skill: `oracle` (see ~/.claude/skills/oracle).

## Anti-patterns to avoid

- **Don't merge the reports.** Two voices is the whole point.
  Keep both reports separately committed.
- **Don't cap the reports.** Length limits would compress
  out the actionable detail. Long is fine; vague is not.
- **Don't soften.** If the answer is "no, I would not fund
  this today", say that — with the reason.
- **Don't paste the other report's findings in.** Each lens
  must stand on its own. Cross-references are added during
  the synthesis pass, not the review itself.
