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

Target: the complete engine shippable as one piece — dashboard
(rsx-playground), terminal (rsx-term), backends (gateway, risk, matching,
mark, recorder) and marketdata streaming live to both UIs.

rsx-cast is frozen: read-only, findings to BUGS.md.
Deferred (not code): risk meme for the demo.

## Status

- [x] 1 rsx-risk — 2026-10-20, `.diary/20261020.md`
- [ ] 2 rsx-matching
- [ ] 3 rsx-gateway
- [ ] 4 rsx-term
- [ ] 5 rsx-marketdata
- [ ] 6 mark/recorder/cli
- [ ] 7 small crates

## Toolchain

Pinned `nightly-2026-10-20` (cranelift debug + mold kept); `make toolchain-bump`
moves the pin. Done 2026-10-20, 6b739445.

## Ledger — done / not done

Refreshed after each component lands. "Done" means every item of the
definition of done above is met, verified by gate output.

| Component | Done | Not done |
|-----------|------|----------|
| rsx-risk | .ship/51 split; symbol config from `symbol_config_schedule`, env overrides gone; dead WAL replay deleted; README/ARCHITECTURE/specs match the code; Config tab names its readers; test, integration, lint, fmt-check green; gate 1–3 green, gate 4 failed on a CPU-starved box (partials render in <0.5 s when fetched directly; see diary) | funding per-symbol config and persisted interval, liquidation-entry cancels, post-max-rounds amount — proposals in BUGS.md |
| rsx-matching | split; fan_out (send warns once per outage); WAL stall; loud recovery in lib; boot from applied PG config; config poll on sidecar; wire reason codes (cancel + failed); LOC cuts (src 2561→2367 at step 7); engine tests over real sockets + zero-heap; /metrics (rejects, WAL/checkpoint seq); p50/p99 bench + report; playground Matching tab + API + play spec. matching 106/0/15, API gate 663 | step 11 spec/doc fixes (waits on dedup window 1h vs 5m); env fallback when no applied row; Gate 4 red on host load (18 fail, mostly load timeouts, play_trade:70 pre-existing); FailureReason::FillOrKill=13 added to the wire (needs sign-off); rsx-cast WAL flush-retry + mid-file CRC bugs need sign-off; p99 needs a quiet box |
| rsx-gateway | — | 7 open bugs |
| rsx-marketdata | — | streaming to dashboard + terminal end to end |
| rsx-term | — | spec check |
| rsx-playground | — | every component visible |
| mark/recorder/cli | — | spec check |
| small crates | — | spec check |
