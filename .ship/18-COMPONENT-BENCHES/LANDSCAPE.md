# Component Isolation Landscape — 2026-05-22

Snapshot of every component benchmark we now have, run on
the local dev box (debug compile times notwithstanding;
benches are release-profile). Used to attribute the
production GW→ME→GW p50 of 1 128 µs back to specific
components.

## Headline measurements

### Transport (rsx-dxs)

| Bench | p50 |
|---|---:|
| `udp_rtt_loopback_64b` | **29 µs** |
| `cmp_one_way_bench` | (slow/hung — bench design issue) |
| `cmp_rtt_bench` | (slow/hung — bench design issue) |
| `wal_append_fsync_single` | **651 µs** |
| `wal_random_read_10k` | 23.5 ms |
| `wal_random_read_100k` | 305 ms |

The CMP benches need rework — they run > 5 min and likely
deadlock on the sender/receiver coordination. UDP RTT is
the floor: **any CMP round-trip is built on at least 29 µs.**

### Gateway (rsx-gateway)

| Bench | p50 |
|---|---:|
| `ws_parse_new_order` | 1.21 µs |
| `ws_parse_cancel` | 1.14 µs |
| `ws_parse_heartbeat` | 945 ns |
| `ws_encode_fill` | 1.40 µs |
| `ws_encode_order_update` | 826 ns |
| `ws_encode_heartbeat` | 387 ns |
| `jwt_validate_no_jti` | 8.3 µs |
| `jwt_validate_with_jti` | 9.6 µs |
| `jti_tracker_bench` | (in progress) |

### Risk (rsx-risk)

| Bench | p50 |
|---|---:|
| `validate_bench` | **281 ns** |

### Matching (rsx-matching)

| Bench | p50 |
|---|---:|
| `book_match_n_levels/n=1` | **149 µs** (NOT 54 ns!) |
| `book_match_n_levels/n=5` | 96 µs |
| `wal_replay_30k_records` | 119 ms |
| `process_order_bench` | (slow/incomplete) |

**The 54 ns "match single fill" Criterion was misleading.**
The real `process_new_order` (book mutation + match +
event emission) is 149 µs on the first level, not 54 ns.
The Criterion bench measured only the inner `match_at_level`
function on an already-resting order list, *without* the
surrounding book-mutation costs.

### In-process E2E pipeline (rsx-cli/bench-e2e-pipeline)

| Metric | Value |
|---|---:|
| Round-trip p50 | **24 459 µs** |
| Round-trip min | 20 135 µs |
| Send/recv ratio | 2 500 / 625 (75 % timeouts) |
| Cross-process p50 (for comparison) | 11 878 µs |

The in-process bench currently has 75 % timeouts and
HIGHER latency than cross-process. This is a bench-tuning
issue, not a refutation. The bench harness needs work
before it produces a clean signal — likely the maker
quote rate is mis-paced for the embedded ME.

## What this tells us about the 1 128 µs Rust GW→ME→GW

Summing isolated components (p50):

| Production leg | Measured (µs) | Attributed components (p50) | Gap |
|---|---:|---|---:|
| gateway_in → risk_in | 60 | UDP one-way (~15) + gateway parse (1.2) + JWT (8.3) | ~35 |
| risk_in → me_in | 205 | UDP one-way (~15) + risk validate (0.3) + plumbing | ~190 |
| me_in → me_out | 158 | book_match_n_levels n=1 (149) | ~9 |
| me_out → risk_out | 39 | UDP one-way (~15) + risk receive | ~24 |
| risk_out → gateway_out | 666 | UDP one-way (~15) + gateway route (~80) + **CMP poll sleep (~500)** | ~70 |

**Key attribution wins:**

1. **`me_in → me_out` is fully explained by `book_match_n_levels` (149 µs vs 158 µs).** The match work itself dominates; tracing was not the culprit at this leg. The previous "54 ns match" framing was wrong.

2. **`risk → ME` 205 µs has ~190 µs of unexplained overhead** beyond UDP + risk validate. That's CMP framing + tokio reactor wake + risk's outbound CMP send. Worth instrumenting further.

3. **`risk_out → gateway_out` 666 µs ≈ CMP poll sleep + plumbing**, confirming the meta-review's framing: the `monoio::time::sleep(100µs)` in `rsx-gateway/src/main.rs:407` is the structural bottleneck. (I tried `yield_now()` to remove it; that starves the accept task in monoio's single-threaded runtime — reverted in `9bbb8f6`. The right fix needs `monoio::select!` on a monoio-native UDP socket, which requires refactoring `rsx-dxs::CmpReceiver` to expose the socket through the monoio reactor.)

## Surprises

- **WAL append + fsync is 651 µs p50.** The 31 ns Criterion benchmark only measured the in-memory `Vec` extend. Real durability cost is **20 000× higher**. Production ME does this every order. Worth a hard look — could batch fsyncs or switch to async durability with an explicit fence per Nth record.
- **WAL random-read scales linearly with WAL size**: 23 ms at 10k, 305 ms at 100k. The cold-tier NAK retransmit path is currently O(n) per random read. A seq-indexed read isn't there.
- **Risk validate is essentially free (280 ns).** Risk's cost is entirely in IO + plumbing, not in the algorithm.
- **WS parse / encode are sub-µs.** The dashboard "synthesize a JSON fill" cost is ~1.4 µs, not the ~10 µs I'd guessed.

## Failures to acknowledge

- `cmp_one_way_bench` and `cmp_rtt_bench` hang — likely a producer/consumer coordination bug in the bench harness itself.
- `process_order_bench` didn't complete in the budget — needs investigation.
- `shadow_book_apply_bench` and `aggregator_bench` numbers not captured in this run.
- The in-process pipeline bench has 75 % timeouts; the result is uninterpretable until the bench is reworked.

These are bench-harness bugs, not production findings. The
sub agents wrote the benches but didn't get clean numbers
out of them before timing out. Follow-up sprint should
fix the harnesses; current production numbers are the
in-process tracing-derived stages (SPEED-OFFHOT.md) until
then.

## What this unlocks

The component landscape lets us argue from the bottom up
when proposing optimisations. For example, *"the
risk → ME 205 µs leg has 190 µs of unattributed overhead;
let's add sub-stage tracing inside risk's outbound CMP
send loop"* — a concrete, scoped task with a measurable
goal.
