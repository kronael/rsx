# 53 — Component completion pass

Founder ask (2026-10-20): take each exchange component, reduce LOC, make every
spec in specs/2 work, show it in the playground, make it complete — including
the gateway and the terminal (rsx-term).

Runs ONE component at a time (code edits are sequential on the shared tree).
Each component ends gate-green and committed before the next starts.

## Per-component definition of done

1. Every spec in specs/2 owned by the component is implemented and tested
   (spec = approved design). A gap that needs a design beyond the spec goes
   to BUGS.md as a proposal, not built.
2. LOC reduced: dead code, duplication, copy-paste blocks gone; no behaviour lost.
3. Playground shows the component's features (dashboard tab / API / play spec).
4. `make test`, `make integration`, `make lint`, `make fmt-check` green;
   playground `make gate` green when playground was touched.
5. Coherent conventional commits, BUGS.md pruned, diary entry.

## Queue

| # | Component | Carries over |
|---|-----------|--------------|
| 1 | rsx-risk | execute .ship/51 split; read symbol config from PG (RISK-IGNORES-SYMBOL-CONFIG), drop env config |
| 2 | rsx-matching | execute .ship/52 split |
| 3 | rsx-gateway | 7 open bugs (dedup window, stranded GTC, reject delivery); crate CLAUDE.md |
| 4 | rsx-term | terminal (Go) vs its specs + playground |
| 5 | rsx-marketdata | 5 open bugs |
| 6 | rsx-mark, rsx-recorder, rsx-cli | |
| 7 | rsx-health, rsx-log, rsx-messages, rsx-types, rsx-book | spec check, LOC |

rsx-cast is frozen: read-only, findings to BUGS.md.
Deferred (not code): risk meme for the demo.

## Status

- [ ] 1 rsx-risk
- [ ] 2 rsx-matching
- [ ] 3 rsx-gateway
- [ ] 4 rsx-term
- [ ] 5 rsx-marketdata
- [ ] 6 mark/recorder/cli
- [ ] 7 small crates
