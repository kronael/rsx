# PROGRESS

updated: Feb 27 18:31:28  
phase: executing

```
[██████████████████████████████] 100%  3/3
```

| | count |
|---|---|
| completed | 3 |
| running | 0 |
| pending | 0 |
| failed | 0 |

## log

- `18:29:42` done: Fix the `./start` Python script: (1) in `build_spawn_plan()` (7 files, +285/-69)
- `18:30:28` done: Fix rsx-risk multi-symbol routing. Read `RSX_ME_CMP_ADDRS` ( (7 files, +290/-70)
- `18:30:29` judge skip: Fix the `./start` Python script: (1) in
- `18:30:30` done: Fix rsx-marketdata multi-symbol subscriptions. Read `RSX_ME_ (7 files, +290/-70)

## assessment

**Overall: ~95% of goal met. One minor count discrepancy; all core routing
logic correct.**

### What was built (all three tasks completed)

**rsx-risk** (`main.rs`):
- `parse_me_cmp_addrs()` reads `RSX_ME_CMP_ADDRS` (comma-sep), falls back to
  `RSX_ME_CMP_ADDR`, defaults to `127.0.0.1:9110`.
- Derives `symbol_id = port - 9100` from each address.
- Builds `me_senders: HashMap<u32, CmpSender>` keyed by symbol_id.
- Routes orders and cancel requests via `me_senders.get_mut(&symbol_id)`.
- Risk replicas also call `parse_me_cmp_addrs()` at startup.

**rsx-marketdata** (`main.rs`):
- Reads `RSX_ME_CMP_ADDRS` / `RSX_ME_CMP_ADDR` fallback.
- Creates one `CmpReceiver` per address, bind port = ME port + 400.
- Event loop iterates all receivers in round-robin.
- Single WS port (8180) and subscription protocol unchanged.

**`./start`** (Python):
- `me_cmp_addrs` computed as comma-joined `9100 + symbol_id` for each symbol.
- Passed as `RSX_ME_CMP_ADDRS` to Risk, Risk replicas, and Marketdata.
- Mark Binance combined-stream URL built from all selected symbols.
- One `("recorder", ...)` entry per scenario with correct env vars.

### Scenario entry counts

| Scenario | Expected | Actual | Match |
|----------|----------|--------|-------|
| minimal  | 6        | 6      | ✓     |
| duo      | 7        | 7      | ✓     |
| full     | 9        | 9      | ✓     |
| stress   | 12       | 11     | ✗     |

Stress discrepancy: spec says 12, actual is 11. Likely cause: stress has
2 GWs, 1 Risk primary + 2 replicas = 3 Risk, 3 MEs, 1 Mark, 1 Marketdata,
1 Recorder → 3+3+2+1+1+1 = 11. The spec's `2×risk` could mean 2 primary
shards (not replicas), which would add one more process and reach 12.
This is ambiguous in the scenario matrix and is the only deviation.

### Quality notes

- Backwards compat (single `RSX_ME_CMP_ADDR`) implemented in both crates.
- Bind port derivation formula (ME+400) is clean and consistent.
- Recorder is single-entry, subscribing only to first symbol's ME —
  acceptable per spec ("one entry per scenario") but not multi-ME aware.
- No changes to `server.py` (playground delegates to `build_spawn_plan`
  unchanged, as required).

### Missing / open

- `stress` spawn count is 11 not 12. If "2×risk" means 2 primary shards
  (each with 1 replica), the start script needs a second primary risk
  process for the stress scenario.
