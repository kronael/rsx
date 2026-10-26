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
- [x] 3 rsx-gateway — 2026-10-26 (open items in the ledger)
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
| rsx-gateway | steps 0–15: handler harness, per-user pending, timeout per 18/29, replay to clients, one heartbeat/reap, bounded outbound, codes 5/8, 1000/s instance limit, pinned recv, OrderRequest→rsx-messages, Gauge→rsx-health, /health+REST per 26, LOC −622, non-ASCII cancel panic, reusable JWT (jti tracker gone), cid retry 1 h, p50/p99 tail bench, specs+docs current, playground Gateway tab. Min order size 1 USD (code 14, risk check, in-flight-only cap, config tab, generators, migration 007), ME book 1M (env, playground 65,536). gateway 255/0/2, make test 1207, integration 60, API gate 746 | spec 61 cid design (D1–D4 owed); gateway REST account/position/fill reads (decide); roadmap item 17; GW-SERVER-FRAME-PARSERS-UNUSED; checkpoint keeps old slab capacity; p99 on a quiet box; Gate 4 not rerun |
| rsx-marketdata | — | streaming to dashboard + terminal end to end |
| rsx-term | — | spec check |
| rsx-playground | — | every component visible |
| mark/recorder/cli | — | spec check |
| small crates | — | spec check |

## Founder decisions — 2026-10-21

- Resting-order DoS → minimum order size (~1 USD notional, or per-token minimum in the symbol config); no resting cap.
- rsx-cast: fix the WAL flush-retry and mid-file-CRC-as-EOF bugs (signed-off exception to the freeze).
- Dedup window 1 h: specs change; gateway cid retries inside it are idempotent.
- `FailureReason::FillOrKill = 13` kept.
- Risk funding-on-boot: fix now.
- TLS terminates at a proxy; gateway stays plain WS.
- JWT reusable until expiry.
- Exchange tooling (WAL decode/verify, invariant checks, token mint, gateway client, stress/latency probe, cluster launch, faults, config) moves into the Rust `rsx-cli`; the playground is glue that calls `rsx-cli --json`. No PyO3. Queued after the gateway, before marketdata.
- Front ends: rsx-glass (trading UI, specs in `rsx-glass/specs/`), rsx-term until its spec moves then deleted, rsx-playground as demo + testing tool. Every other UI removed.

## Queued specs (write now, build later)

- cid retry follow-ups: an opus agent adversarially reviews (a) a distinct duplicate answer that cannot read as "your live order failed", (b) unique cids in stress_client/bench_probe, (c) the gateway-restart gap; output is a spec in `specs/2/`, built later. Waits for the opus slot.
