# Trust boundary: the ME does not re-validate

**Domain term.** *Validation* is checking that an order is well-formed
(prices/quantities in range, known symbol) and permitted (the user has
margin, the order obeys position limits). A *trust boundary* is the line
past which code assumes its inputs were already checked by someone else.

## Problem

The obvious-but-wrong instinct is "defense in depth": re-check the order
inside the matching engine too, just in case. That looks safe. It is not
free and it is not correct here:

- **Cost.** Every re-check adds branches to the hot path — the measured match
  and accept paths (`notes/depth-independent.md`) would carry
  work that produces no new decision.
- **Correctness.** It creates a *second owner* for a concern that already
  has one. Two owners drift: a limit tightened in risk but not mirrored
  in ME, or vice versa, silently diverge. The repo's single-owner rule
  (`../CLAUDE.md` "Trust boundaries") exists precisely to stop this.

## Fix

By the time an order reaches this process it has passed two upstream
gates, each the sole owner of its concern:

```
client ──▶ Gateway ──▶ Risk[user] ──▶ ME[symbol]
           JWT/TLS +    margin +        assumes well-formed,
           structural   position        in-shard; matches only
           well-formed   (pre-trade)     on limit price
```

The matching engine assumes the caller is authenticated, authorized, and
risk-approved. It does not repeat margin or position-limit policy on the hot
path. `rsx-book::process_new_order` still applies the structural sign, tick,
and lot checks required to keep book state valid; those are book invariants,
not duplicated user-policy validation. The one thing ME *is* strict about is its own
*output* — every fill-path WAL append crashes rather than drop a record
(`notes/authoritative-wal.md`), because ME is the authoritative writer of
fills and a lost fill is unrecoverable. Input trust and output paranoia
are not in tension: ME trusts what it is told and guarantees what it
emits.

## Cost it removes

Duplicate validation logic and its hot-path branches, plus the class of
bugs where two owners of one rule fall out of sync.

## Note for auditors

"ME does not repeat authentication, authorization, or margin checks" is
**not** an actionable finding — it is the documented design. Those concerns
are owned upstream; structural book invariants remain enforced here
(`specs/2/47-validation-edge-cases.md` maps every check to its layer;
casting is unauthenticated by design per `specs/2/4-cast.md` §10.4).
Adding a check here to "harden" the ME re-opens a settled boundary; the
fix for a genuinely missing check is in the gateway or risk tile, not
here.

## Cite

- `../CLAUDE.md` "Trust boundaries (read this before adding 'security')";
  `ARCHITECTURE.md` § "Trust Boundary".
- `specs/2/47-validation-edge-cases.md` (owner map);
  `specs/2/45-tiles.md` §3.1.
