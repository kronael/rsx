# CLAUDE.md — docs/concepts/

The **why** layer. Each page explains *why* a load-bearing design choice
is the way it is — or names a deliberate tradeoff. Not *what* the code does
(that's `specs/2/`), not *what-is-now* structure (that's `ARCHITECTURE.md`),
not *how fast* (that's the README bench tables + `reports/`), not narrative
(that's `blog/`). Concepts are the cross-cutting design rationale.

## One page = one concept

- One concept per file, `kebab-case.md` named by the concept.
- Adding a page: add a one-line entry to `index.md` (concept — one-sentence why).
- Keep it short. If a page runs long, it's probably two concepts, or it drifted
  into how-it-works (move that to ARCHITECTURE).

## Shape

```
# Title

Lead paragraph: state the concept and the tradeoff it makes, in 1–3 sentences.
The reader should know the point before the first heading.

## <the mechanism, in plain terms>
## <the tradeoff / what you give up>
## <the failure mode, when relevant>  ("why an unpinned spinner is dangerous")

---

Deeper: [blog/NN-x.md](../../blog/NN-x.md), [specs/2/NN-x.md](../../specs/2/NN-x.md)
```

The `Deeper:` footer is mandatory — link the blog post(s) and spec(s) that go
further. Paths are `../../`-relative from here.

## Voice — no fluff

- Terse, plain narration. Lead with the point; cut throat-clearing.
- Load-bearing claims carry a **number** (ns, µs, MB) or they don't belong.
- Name the tradeoff honestly — "you spend one whole core to buy determinism."
  A concept with no cost stated is marketing, not a concept.
- **What-is-now only.** No history ("was X, now Y"), no dates, no marketing
  adjectives ("blazingly", "seamless"), no "rollout" heading.
- Numbers must match the current README/PROGRESS/ARCHITECTURE. If two docs
  disagree, reconcile to the authoritative source — don't invent a third.

## Consistency with the rest of the repo

- The matching engine is a **process**, not a tile (a tile is a thread inside
  a process). Gateway and marketdata are **monoio** (I/O-bound), not tiles.
- It's a **derivatives** exchange (perpetuals are the product built today;
  options and SFDX are pending) — never "perpetuals exchange" as the descriptor.
- Cite `11-glossary.md` terms rather than re-defining them inline.
