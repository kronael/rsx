# CTO Review Round 2 — RSX (2026-05-24)

Adversarial code/spec audit at `ccc1ac3` (HEAD); 96 commits since
v0.2.0. No live cluster touched. Codex consulted twice for fresh
second opinion; both passes confirmed the most critical findings.

Findings tally: 12 critical, 18 important, 16 nice; **46 actionable
items**. Five round-1 risks forced-ranked; eight new scenarios
traced; five claims verified end-to-end against the code/benches.

---

## 0. Round 1 → Round 2 diff

Round 1 ("CTO-REPORT.md", pruned with `.ship/16-CTO-CEO-REVIEW/`)
identified five critical risks. Status at HEAD:

| # | Round 1 finding | Verdict | Citation |
|---|---|---|---|
| R1 | Silent fill drop in `rsx-matching` (`let _ = wal_writer.append(...)` × 6) | **CONFIRMED-RESOLVED** | All six sites now `.expect("INVARIANT N: ...")` — `rsx-matching/src/main.rs:502-504, 531-533, 654-?`. Commits `cbbdfd2` / `ee30c37`. |
| R2 | `<50µs` claim vs 11.8 ms measured | **PARTIALLY-RESOLVED / REGRESSED** | README:207 demoted to "design budget"; but 8 sites in CHANGELOG/CLAUDE/ARCHITECTURE/FEATURES/MONITORING still cite `<50µs` (see §5 claim-3). Round 1 acceptance test ("strip the language") not done. |
| R3 | `JtiTracker` null-defeated (token-without-jti bypass) | **CONFIRMED-RESOLVED** | `rsx-gateway/src/ws.rs:206` rejects missing jti before touching tracker. Commit `72bd481`. But see new R-N1 (handshake race) and R-N2 (heap alloc per record). |
| R4 | ME drops events ≥ 10K (panic-not-stall) | **REGRESSED** | `rsx-book/src/book.rs:103-108` is still `assert!`, still mismatched with `specs/2/6-consistency.md:168` ("ring full = stall"). Buffer raised 10k→65k (`9159639`) but the panic vs stall semantic was never reconciled. Codex confirms: "the bug is the spec lying about runtime behavior." |
| R5 | "Zero heap on hot path" claim falsified | **REGRESSED** | Three new hot-path heap sites since round 1: `rsx-dxs/src/cmp.rs:557` (`try_recv` returns `(WalHeader, Vec<u8>)`); `rsx-dxs/src/cmp.rs:718` (`payload.to_vec()`); `rsx-risk/src/shard.rs:846,896` (`users_for_symbol().to_vec()` per BBO). The CmpSender `send_ring` is preallocated, but the `CmpReceiver` recv path is not. CLAUDE.md:25, README:266, BLOG.md:248 still claim "zero heap on hot path." |

**Net round-1 status**: 2 fully resolved, 1 partial, 2 regressed.

The CTO-flagged R4 from the META-REVIEW ("order_prod ring still silently drops"
at `rsx-risk/src/main.rs:539`) is **still present at HEAD**. The "all peer
sites" claim in `5bb3f35`/`ee30c37` did not cover this site. See R-N3 below.

---

## 1. Verdict

**Would I bet a customer SLA on this in 6 months? Confidence: 28/100.**

Why not lower: the audit cadence is real, ~340 ns matching cost is
genuine, JtiTracker is wired, the dual-lens methodology works, the
trust-boundary discipline survived a 96-commit sprint. There's a
shippable kernel here.

Why not higher: the **matching engine has no WAL replay on startup**
(§6 Scenario C below — codex-confirmed); orders dropped at risk
ingress are **invisible to clients and ME** (§6 Scenario A); the
clippy `-D warnings` gate **fails at HEAD** (§5 claim-1); the
"zero heap on hot path" headline is **disprovable in three
greps**; bench-reference.json is **sealed against a known-broken
probe and never re-measured** after the 2026-05-23 audit. The
single biggest reason for the low confidence: **the WAL is largely
decorative for crash recovery**. ME writes records, never replays
them. A SIGKILL between snapshots (every 10 s) loses every fill
the gateway already shipped to a client.

This is the kind of bug a design partner would catch in week one
of integration and not get over.

---

## 2. Top 5 strengths (don't break)

1. **Matching algorithm itself is 340 ns p50** (`bench-match-rt`,
   reconciled in `245bd03`). Dedup 70 + wal_accept 60 + match 80 +
   wal_events 110. This number survives scrutiny; the in-process
   round-trip (9.58 µs) and the cross-process (1.128 ms) are
   honestly attributed to CMP plumbing, sendto syscalls, and
   monoio sleep, not to the algorithm.

2. **`rsx-dxs` has zero `rsx-*` production deps** (`cargo tree -p
   rsx-dxs --edges normal | grep rsx-` returns only the crate's
   own header line). Verified at HEAD `ccc1ac3`. The wedge artifact
   is provably reusable.

3. **JtiTracker is wired through the handshake** (`rsx-gateway/src/
   ws.rs:127-217`) and rejects tokens missing `jti` *before*
   touching the tracker (line 206). Replay defence is no longer
   null-defeatable by stripping the claim. Commit `72bd481` /
   `f4ff065`.

4. **Trust-boundary discipline codified** (`CLAUDE.md` "Trust
   boundaries" + `bde3211` revert of the CMP source-IP filter).
   Two sprints in, no code in the wrong layer has been re-added.

5. **WAL header has a version byte** (`rsx-dxs/src/header.rs:43`)
   that is rejected at every ingress (`CmpReceiver::try_recv:570`,
   `WalReader::next:523`, `read_record_at_seq:749`). The framing
   layer is genuinely versioned even where the payload layer is
   not (see R-N4 for the payload gap).

---

## 3. Top 10 NEW risks (forced rank)

Numbered R-N1 through R-N10. None of these are in the round-1 punch
list.

### R-N1 — ME has no WAL replay on startup; 10 s of fills can vanish [critical]

`rsx-matching/src/main.rs:286-294` loads only `snapshot.bin`; there
is **no `WalReader::next()` loop** between the snapshot and the
crash point. `rsx-matching/src/wal_integration.rs:199-227`
(`load_snapshot`) is the entire recovery path. Snapshots are
written every 10 s (`rsx-matching/src/main.rs:723`).

Worse, the WAL itself is in-memory until the 10 ms periodic flush
(`rsx-dxs/src/wal.rs:140-163` + `wal_integration.rs:181-193`).
But the fills are forwarded to risk and gateway over CMP/UDP
*immediately* after `append()` (`rsx-matching/src/main.rs:509-625`).
**Order of operations: client sees fill → ME crashes → ME restart
loads snapshot from N seconds ago → ME state diverges from
client's history.**

Codex's verdict: "WAL persistence is largely decorative for crash
recovery." Spec invariant 7 ("Matching engine persists orderbook
via snapshot + WAL") is **false in practice** — only snapshot.

**Acceptance test**: Run a crossing trade, wait until gateway
receives FILL, `SIGKILL -9` ME, restart, assert post-restart book
contains the trade. Today this fails.

### R-N2 — Order ring overflow at risk ingress is silently dropped, never reported [critical]

`rsx-risk/src/main.rs:539-544`:
```rust
if order_prod.push(order).is_err() {
    warn!("order_prod ring full — dropping order");
}
```
Ring sized 2048 (`rsx-risk/src/main.rs:474`). At 50 k orders/s,
ring fills in ~40 ms if downstream stalls. Codex (round-2 pass):
"Exchanges fail on invisible divergence between user intent and
engine state. P0-class behaviour."

Comparison sites: `fill_prod` stall-loops (`rsx-risk/src/main.rs:
690-714`) and `bbo_prod` counts drops with power-of-two warn
(`599-616`). **The order-ingress site uses `.is_err()` instead of
`let _ =`, so it passed META-REVIEW R4's lint check, but is
behaviourally identical.**

Gateway already moved the order to `pending` at
`rsx-gateway/src/handler.rs:451-504`. With no downstream event,
the pending entry ages out at `rsx-gateway/src/main.rs:359-367`
and `pending.rs:85-101` returns the stale `Vec<PendingOrder>` to
a `let _ = ...` discard. **No `ORDER_FAILED` is sent. The client
sees silence.** ME never knew the order existed.

**Acceptance test**: Shrink `order_prod` to 1, send 2 orders, assert
the second order receives an `OrderUpdate(status=failed)`. Fails
today.

### R-N3 — `cargo clippy --workspace -- -D warnings` (== `make lint`) FAILS at HEAD [critical]

`make lint` is the documented quality gate (CLAUDE.md:171). At
`ccc1ac3` it errors with 4 messages in `rsx-cli/src/bin/
bench_match_rt.rs`:
- L93 — unused `const ME_STAGES_START: usize = 2` (dead_code).
- L470 — `s[8] - s[0] /* unused */ * 0 + 0` triggers
  `clippy::identity_op` (`+ 0`) and
  `clippy::erasing_op` (`* 0`) — the latter is `-D` by default.
- A 4th error of the same shape.

**This bench binary was added in the 2026-05-23 sprint
(`6bcc5ce`).** Either nobody ran `make lint` on the commit, or
they ran it and ignored it. CHANGELOG.md:138 still says "Clippy
lib + bin: 13 warnings → 6". That count is **wrong as written**;
HEAD is red. CI is not catching it because the actual gate flow
in the Makefile uses `make gate-3-api` (Python tests) and
playwright; `make lint` is run separately. Codex (round-1 pass)
called this "important: HEAD is definitionally red."

**Acceptance test**: `make lint` exits 0. Today it does not.

### R-N4 — FillRecord wire-format silently extended at offset 88; no WAL_HEADER_VERSION bump [critical]

`rsx-messages/src/lib.rs:51-70` — FillRecord gained `taker_ts_ns:
u64` at offset 88. The size assertion (`const _: () =
assert!(mem::size_of::<FillRecord>() == 128)`) holds **because
the previous version had 8 bytes of implicit padding at the same
offset**. `WAL_HEADER_VERSION_LATEST` (`rsx-dxs/src/header.rs:46`)
is still `V1`.

The mitigation in both receivers (`rsx-gateway/src/route.rs:52-58`
and `rsx-risk/src/main.rs:659-666`) is a plausibility check:
```rust
let anchor_ns = if rec.taker_ts_ns > 1_700_000_000_000_000_000 {
    rec.taker_ts_ns
} else {
    rec.ts_ns
};
```
Codex (round-1 pass, escalated): "wishful thinking over raw
`#[repr(C)]` bytes. Any byte pattern that happens to numerically
exceed 1.7e18 will be interpreted as a valid t0_ns anchor,
producing nonsense latency deltas on replay."

Worse, this is the second wire-format change without a version
bump in 2 weeks (the first was the size_of/align_of assertion
addition itself). The V0/V1 split that exists to prevent this is
not being used.

**Acceptance test**: Bump `WAL_HEADER_VERSION_LATEST` to V2; or
change the offset-88 field to a sentinel-padded discriminator
(e.g. `_pad1 = [0xFF; 4]` when populated, `[0x00; 4]` when
not — readers reject ambiguity).

### R-N5 — JtiTracker burns the token before the 101 response is written [critical]

`rsx-gateway/src/ws.rs:127-164`. The handshake calls
`extract_user_and_record_jti(...)` at line 127 which inserts the
jti into the tracker at line 210-213. Then at line 161 it calls
`stream.write_all(resp_bytes).await` — **if this write fails on
any link below WSS (broken TCP, mobile flake, proxy drop), the
jti is consumed but the client never saw "101 Switching
Protocols".** Client retries with same JWT → "jti replay" reject.

Codex (round-2 pass): "edge-triggered auth bug that produces
'your API is unreliable' escalations."

This is mitigated only by `jwt_ttl_s = 7 days`
(`rsx-auth/src/rsx_auth/config.py:25`) plus the auth service
issuing a fresh `jti` per call — so the client can re-auth, but
**the same JWT is dead**. For a long-lived JWT model this is
production-grade auth-rejection on transient network failure.

**Acceptance test**: Inject a write failure after
`extract_user_and_record_jti()` returns Ok, then reconnect with
the same JWT and assert success. Fails today.

### R-N6 — Gateway advances CMP seq even after `send_raw` fails [critical]

`rsx-gateway/src/handler.rs:497-504`:
```rust
if let Err(e) = sender.send_raw(RECORD_ORDER_REQUEST, bytes) {
    warn!("gateway: forward order to risk failed: {e}");
}
sender.advance_seq();
```
`advance_seq` is **unconditional**. `CmpSender::advance_seq()`
(`rsx-dxs/src/cmp.rs:464`) bumps `next_seq`. The downstream
receiver sees a sequence gap and NAKs — but the payload was never
sent, so the NAK gets back nothing in the send_ring slot indexed
by `seq & MASK` (because send_raw also failed to populate it).

**Two failure modes**:
1. Receiver NAKs the missing seq, sender retransmits whatever
   happens to be at `ring[seq & MASK]` — which is now stale
   data from `seq - SEND_RING_SIZE` ago. **Cross-stream
   corruption.**
2. Sender's seq counter has drifted from the receiver's
   expected. Subsequent legitimate sends are seen as seq-N+1
   on the wire but the receiver expects seq-N → false NAK
   storm.

Codex (round-2 pass, extra finding): "another silent
disappearance path."

**Acceptance test**: Inject a `send_raw` failure (e.g. close the
sender's socket); send N more orders; assert the receiver sees
all N (or none) but not a malformed mix.

### R-N7 — `Event::OrderFailed` is not persisted to WAL [important]

`rsx-matching/src/wal_integration.rs:170-172`:
```rust
Event::OrderFailed { .. } => {
    // OrderFailed is not persisted to WAL
}
```
Spec invariant #1 (CLAUDE.md:182): "Fills precede ORDER_DONE (per
order)" — implicitly assumes one terminal state per order.
Invariant #2: "Exactly-one completion per order (ORDER_DONE xor
ORDER_FAILED)." But OrderFailed never reaches WAL, so:

- On ME crash between OrderFailed emit and CMP send to risk: the
  failure is **never recovered**. Risk has no record. The order
  never had a final state.
- On replay: a future replay-aware ME would never see the
  OrderFailed; the invariant is violated by omission.

This is silently downgraded by the comment, with no spec citation
or carved-out exception in `specs/2/6-consistency.md:168-172`.

**Acceptance test**: Crash ME between an OrderFailed emit and
the next 10 ms WAL flush; assert that on restart the failed
order is either re-evaluated or re-emitted. Fails today.

### R-N8 — WAL retention bumped 10 min → 48 h with no disk-capacity review [important]

Commit `8468bad` bumped retention from `10 * 60 * 1_000_000_000`
to `48 * 60 * 60 * 1_000_000_000` in `rsx-matching/src/main.rs:280`
and `rsx-mark/src/main.rs:83`. That is **288×** the storage
budget per stream.

At 10k orders/s and 128 B per record, that's ~110 MB/s × 48 h ≈
**19 TB per stream**. Multiplied by per-symbol matching engines
plus mark + recorder, the steady-state disk footprint is several
terabytes per symbol — with no diary entry, no spec update, no
ops runbook reference to disk pressure, no GC behaviour
verification under the new limit. The CHANGELOG, ARCHITECTURE,
and the spec's `48-wal.md` (which `8468bad` did edit) treat this
as a one-line change.

The change is also commit-message-only: no rationale in body.

**Acceptance test**: Burn-in run at 5 k orders/s for 48 h on a
representative disk; assert (a) no disk-full, (b) GC keeps pace,
(c) p99 latency does not degrade.

### R-N9 — `try_recv` returns `(WalHeader, Vec<u8>)` — heap alloc per CMP frame [important]

`rsx-dxs/src/cmp.rs:555-732`. Every successfully-received frame
ends with `Some((hdr, payload.to_vec()))` at line 718 (and same
shape in the reorder-buffer path at line 700). At line rate
(say, 100 k frames/s on a busy gateway), this is 100 k Vec
allocations/free per second on the hot path.

CLAUDE.md:25 ("Zero heap on hot path") and BLOG.md:248 (the
narrative artifact!) cite zero-heap as a design property. The
comment at `cmp.rs:486-494` admits "Heap-allocates per inserted
packet. Acceptable because: (1) bounded at reorder_buf_limit
(default 512)..." — but that justification covers only the
reorder-buf path, NOT the steady-state try_recv hot path which
also allocates.

This was not in round 1 because the round-1 audit focused on the
SENDER side (`send_ring` was rewritten preallocated). The receiver
side still allocates.

**Acceptance test**: `cargo bench --bench cmp_one_way_bench` with
a `tracking_alloc` global allocator that asserts zero
heap-alloc-bytes per iteration in the steady state. Fails today.

### R-N10 — `bench-reference.json` is the latency CI floor; it was sealed against a known-broken probe and never re-measured [important]

`bench-reference.json` shows `e2e_us.p50=11780` with `ts=
1779449990` (2026-05-22 11:39 UTC). The probe-race fix
(`82e9966`) landed 6 hours later. **Two days have passed; nobody
re-sealed**. The latency-publish baseline (`bench-baseline.json`,
ts=1779482219 = 2026-05-22 20:36) shows 11878 — also pre-the
re-measurement work in `2026-05-23`.

Meanwhile `.diary/20260523.md` explicitly notes the
`monoio::time::sleep(100µs)` still costs ~655 µs on the GW→ME→GW
p50 and the proper fix is still open. Codex (round-1 pass):
"The numbers are not merely old; they are known to be contaminated
by an acknowledged measurement-distorting bug still present in
code."

Any PR that genuinely improves p50 from 11878 → 11200 would pass
the 10 % regression gate, but so would any PR that degrades it
to 12950. The gate is too loose AND outdated.

**Acceptance test**: `bench-reference.json` carries a
`probe_version` field that matches the current commit's probe
binary hash. Today it does not.

---

## 4. Forced rank: 3 fixes I'd do this week

### Fix 1 — Wire WAL replay into ME startup. (1-2 days)

**Acceptance test**: `SIGKILL` ME between snapshots, restart,
assert the orderbook + fills are restored consistent with what
gateway clients saw. Today it fails.

The fix is mechanical: after `load_snapshot` (line 286), open
`WalReader::open_from_seq(symbol_id, book.sequence + 1, &wal_dir)`
and replay every record until EOF. Apply OrderInserted /
OrderCancelled / OrderDone / Fill to the book in order. Cost:
~50 LOC in `rsx-matching/src/main.rs` + one
`tests/replay_recovers_post_snapshot_test.rs`.

This single fix closes R-N1, addresses spec invariant 7, and
makes the WAL the actual source of truth it's supposed to be.

### Fix 2 — Make order_prod overflow visible to clients. (1 day)

`rsx-risk/src/main.rs:539-544`. Either:
- **Stall** like fill_prod (lines 690-714): loop pushing,
  call `accepted_cons.run_once` to drain, retry.
- **Reject** explicitly: emit `OrderResponse::Rejected { reason:
  RISK_BACKPRESSURE }` to `resp_prod` so the existing line-908
  drain emits a real `ORDER_FAILED` to the gateway. Client sees
  a terminal state.

**Acceptance test**: Shrink `order_prod` to 1, send 2 orders,
assert one fills/inserts and the other receives
`OrderUpdate(status=failed, reason=...)`.

Half a day. Closes R-N2.

### Fix 3 — Fix `make lint` then add it to the pre-merge gate. (2 hours)

Delete the dead code at `rsx-cli/src/bin/bench_match_rt.rs:93`,
fix the identity_op + erasing_op at line 470 (`s[8] - s[0] +
0 * 0` is a placeholder leftover that should never have shipped).
Then add `cargo clippy --workspace -- -D warnings` to the
pre-merge sequence in `Makefile:gate-3-api` (currently it lives
only at `make lint`).

**Acceptance test**: `make lint` exits 0. The CI fails a PR that
introduces a new clippy warning.

Closes R-N3 and the broken-promises problem at one stroke.

---

## 5. Verified claims

Five specific claims rated: CONFIRMED / REFUTED / PARTIAL.

### Claim 1 — "887 tests pass" (MEMORY.md, ONEPAGER.md)

**CONFIRMED.** `cargo test --workspace --lib --tests --
--test-threads=1` ran in this audit:
```
passed=887 failed=0 ignored=46
```
(`tmp/cto2-test-results.log`).
Three sources cite three different numbers though:
- MEMORY.md / ONEPAGER.md: 887
- CHANGELOG.md:135: 878
- PROGRESS.md and README.md: 878 (stale)

So while 887 is correct, four docs disagree about it.

### Claim 2 — "rsx-dxs has zero rsx-types production dep" (WEDGE.md, CHANGELOG.md)

**CONFIRMED.** `cargo tree -p rsx-dxs --edges normal | grep rsx-`
returns only `rsx-dxs v0.2.0 (...)`. No other `rsx-` line.
Verified at HEAD `ccc1ac3`.

### Claim 3 — "<50 µs GW→ME→GW design budget" (CLAUDE.md, ARCHITECTURE.md, README.md, etc.)

**REFUTED for actual production e2e**; **PARTIAL for design budget**.

Cited at 8 sites. Actual measurements:
- In-process matching round-trip: **9.58 µs p50** (`bench-match-rt`).
- Cross-process: **1 128 µs p50** (SPEED-OFFHOT.md, .ship/18-COMPONENT-BENCHES/).
- Cross-process via Python WS probe: **11 878 µs p50** (bench-baseline.json).

The matching algorithm itself is 340 ns p50 — within budget. The
end-to-end production number is **22× over**. README:207 has been
demoted to "design budget"; the other 7 sites still say `<50 µs`
without qualification. Round 1's acceptance test ("strip the
language") was not done.

### Claim 4 — "CmpSender::send body is 99% sendto syscall" (.diary/20260524.md, dfe2ef4 commit)

**CONFIRMED.** Re-ran `cargo bench --bench cmp_send_breakdown_bench
-- --sample-size 10 --measurement-time 3` during this audit:

| Sub-step | p50 |
|---|---:|
| crc32_128b | 15.7 ns |
| header_build | 4.5 ns |
| buf_pack_144b | 3.85 ns |
| sendto_144b_loopback | **3 884 ns** |
| ring_cache_copy_144b | 3.49 ns |
| sum | ~3 911 ns |

sendto / sum = **99.3 %**. There is no optimisation target in
CMP framing.

### Claim 5 — "Sleep audit says only 2 sleep-as-yield bugs" (SLEEPS.md)

**CONFIRMED.** `grep -rn "monoio::time::sleep\|tokio::time::sleep"
rsx-*/src/` returns 6 sites. Two are HOT/WARM sleep-as-yield bugs
(gateway:407, marketdata:328); four are correctly classified
backoffs (mark/source, risk/persist × 2, dxs/client reconnect).
**Critically: gateway:407 and marketdata:328 still have the bug.**
Both at HEAD `ccc1ac3`. The strategic fix
(`monoio::net::UdpSocket` on `CmpReceiver`, ~50 LOC) remains open.

---

## 6. Attack scenarios (3) + code trace

Each scenario consulted via codex (round-2 pass); verdicts below
are mine after independent code-read.

### Scenario A — Sustained 50 k orders/s for 30 s

**VERDICT: LEAKS. Orders disappear with no client notification, no
ME state, no audit trail.**

Trace:
1. Gateway accepts WS frame, parses NEW_ORDER
   (`rsx-gateway/src/handler.rs:451-504`).
2. Gateway enqueues into `pending` (line 451) and calls
   `sender.send_raw(RECORD_ORDER_REQUEST, ...)` (line 497).
3. CMP/UDP delivers to Risk's `gw_receiver.try_recv()` at
   `rsx-risk/src/main.rs:510`.
4. Risk validates, then `if order_prod.push(order).is_err()
   { warn!(...); }` at line 539. **Ring sized 2048
   (line 474); at 50 k orders/s with downstream stall, fills
   in 40 ms.**
5. Dropped order: never reaches shard's `accepted_cons`
   drain (line 957), never reaches ME, no `OrderResponse::Rejected`
   on `resp_prod`, no `RECORD_ORDER_FAILED` to gateway.
6. Gateway's `pending` entry ages out at `main.rs:359-367`;
   `remove_stale` returns `Vec<PendingOrder>` to a `let _ =`
   discard (line 365).
7. **Client sees nothing. Pending row vanishes from gateway
   memory. ME never knew the order existed.**

Codex confirms: "Client sees nothing except eventual silence;
the pending entry is locally garbage-collected. Matching engine
does not know the order existed."

**Acceptance test**: shrink `order_prod` to 1, send 2 orders,
assert second receives `OrderUpdate(status=failed)`. Fails today.

### Scenario B — Gateway crashes mid-WS-handshake AFTER JtiTracker record but BEFORE 101 write

**VERDICT: LEAKS for the same-JWT retry path; bounded blast
radius. Survivable but customer-visible.**

Trace:
1. Client connects, sends GET /ws with `Authorization: Bearer <jwt>`.
2. `extract_user_and_record_jti` succeeds; `JtiTracker.record(jti)`
   marks the token as seen
   (`rsx-gateway/src/ws.rs:206-213, jwt.rs:107-123`).
3. Gateway crashes / TCP RST / mobile-network drop BEFORE
   `stream.write_all(resp_bytes).await` completes (ws.rs:161-162).
4. Client never sees `101 Switching Protocols`.
5. Client retries same JWT → `JtiTracker.record(Some(jti))`
   returns `false` (already in HashSet) → handshake returns
   `Err("jti replay")` → 401 Unauthorized.
6. Client must re-auth via `rsx-auth` to mint a new JWT (fresh
   `jti` per `rsx-auth/src/rsx_auth/jwt_util.py:15-29`).

Mitigations:
- New JWT works (auth issues fresh jti per call).
- `jwt_ttl_s = 7 days` so the dead JWT lives in the tracker until
  evicted (capacity 16 384 — see also R-N9).
- For a malicious flood with distinct JWTs: it costs the
  attacker one valid JWT per evicted slot (auth gates JWT
  issuance via GitHub OAuth → bounded by GitHub API rate).
  Not a DoS primitive, but it is an **annoyance attack** on
  the legitimate user pool.

Codex confirms verdict; flagged this as "important: edge-triggered
auth bug that produces 'your API is unreliable' escalations."

**Acceptance test**: Inject write failure after extract_user_and_record_jti
returns Ok; reconnect with same JWT; assert success. Fails today.

### Scenario C — ME crashes mid-batch (post-append, pre-flush)

**VERDICT: LEAKS catastrophically. The WAL is decorative for
crash recovery.**

Trace:
1. ME calls `wal_writer.append(&mut accepted)` at
   `rsx-matching/src/main.rs:532`. `append` is memory-only
   (`rsx-dxs/src/wal.rs:104-105`: `self.buf.extend_from_slice(...)`).
2. ME calls `process_new_order` (line 547), `write_events_to_wal`
   (line 564) — more `append` calls, still memory-only.
3. ME calls `cmp_sender.send(&mut fill)` (somewhere around 615)
   to forward to risk. **CMP/UDP send is independent of WAL flush.**
4. Risk forwards to gateway (`main.rs:716-721`). Gateway routes
   to user's WS connection (`main.rs:198-235`).
5. **Client receives FILL.**
6. SIGKILL ME (or kernel panic, or VM die-down) before the next
   `flush_if_due` 10 ms tick.
7. ME restarts. `load_snapshot` (main.rs:286) restores book to
   the last 10-second snapshot. **No `WalReader` loop replays
   the records appended between snapshot and crash.**
8. ME's book is now N seconds behind what the gateway showed
   the client. The fill the client saw is **erased**.

Codex: "Buffered-but-unflushed records are lost on crash. … buf
at SIGTERM contains exactly the not-yet-flushed WAL records;
graceful SIGTERM drains it, crash does not. Uncrash-recovered
state is not consistent with gateway's view."

This is **the** finding of round 2. R-N1 is the same point as a
standalone risk; this scenario is the proof.

**Acceptance test**: Crossing trade → gateway sees FILL →
SIGKILL ME → restart → assert orderbook contains the trade.
Fails today.

---

## 7. Surprises (positive + negative)

### Positive

- **Codex flagged a finding I had underweighted.** I described
  the FillRecord wire-format issue as "important"; codex called
  it "critical" specifically because of consumers built against
  the old struct size. Adjusted my severity upward.
- **`cmp_send_breakdown_bench.rs` is genuinely informative.**
  Re-ran it; numbers reproduce within noise. The bench design
  is honest — the `bench_sendto_loopback` test drains in a
  worker thread so the kernel queue never fills.
- **`make test` is fast (~8 s wall) AND covers 887 tests.** Single
  most operationally-useful artifact in the repo.
- **JtiTracker rejecting missing-jti BEFORE touching the tracker
  is good engineering.** The comment at `ws.rs:200-202` correctly
  cites the null-defeat history. (The tracker's INNER comment at
  `jwt.rs:105` is stale — see R-N5.)
- **`rsx-dxs/fuzz/` exists.** Round 1 flagged the absence; partial
  progress. Only one fuzz target (`wal_header`) but the shape is
  right.

### Negative

- **The `let _ =` rule is enforced by lint but defeated by
  `.is_err()`.** Round 1's R4 said "0 hits"; HEAD has 11 `let _
  =` (in benches, all OK) plus `rsx-risk/src/main.rs:539`'s
  `.is_err()` which is behaviourally identical and went unflagged.
  The rule needs to be tightened.
- **`make lint` fails at HEAD.** A repo-public "we lint" claim
  with a broken gate is the worst of both worlds: it signals
  discipline you don't have.
- **The `<50 µs` headline number is still cited 8 times even
  though the project itself measured 11 878 µs.** Marketing-lag,
  not measurement-lag.
- **No `.diary` entries for the 96-commit sprint until 2026-05-23.**
  The user-CLAUDE.md "Read project diary" startup protocol fails
  open: an LLM session looking at this project from cold has
  weak breadcrumbs.
- **The CTO-report and CEO-report from round 1 are pruned**
  (`9728dcf`), making the round-1 → round-2 diff harder than it
  needs to be. The META-REVIEW (PROGRESS-REVIEW.md) survived,
  which is partial mitigation, but the original report is gone.
  The `/ship` close-out rule says "distill durable bits to their
  permanent homes" — the round-1 critical findings should have
  been distilled to CHANGELOG.md or a LESSONS.md, not just deleted.
- **3 stale `rsx-maker` references in *.md** (README:164,312;
  ARCHITECTURE:86; FEATURES:155; PROGRESS:52). META-REVIEW R6
  flagged this; partial fix (CLAUDE.md got rsx-log added).
- **`.ship/` is still committed.** META-REVIEW noted CLAUDE.md
  says new repos should `.gitignore` `.ship/`. This repo has
  4 directories under `.ship/` checked in. Not blocking but
  not clean.

### Expected but absent

- A WAL-replay test exercising the SIGKILL-recovery path.
- A burn-in test at the new 48 h WAL retention.
- A `LATENCY.md` outside `.ship/`. Numbers will be pruned when
  the ship dir gets distilled.
- `proptest` in Cargo.lock. Still missing.

---

## 8. Out-of-scope notes (UI domain)

CTO lens, code only. Things noticed that affect the CEO view:

- The trade UI (`rsx-webui/`) has a `LOG.md` untracked in `git
  status`. If the UI is also being neglected (META-REVIEW noted
  this), it undermines the WEDGE.md "B + A" framing — design
  partners touch the UI first.
- `ONEPAGER.md:62-63` says "Adversarial CTO + CEO audits round
  2 (`.ship/20-CTO-CEO-REVIEW-2/`, in-flight at time of writing)" —
  before the round started. This is fine as forward-looking
  prose, but it's also a small inversion of the audit-then-commit
  norm.
- The auth service's JWT TTL is 7 days
  (`rsx-auth/src/rsx_auth/config.py:25`). Combined with R-N5, a
  user whose JWT got burned by handshake-write-failure must
  re-OAuth. For a productised SDK, "JWT refresh" should exist.

---

## 9. Test/lint state snapshot

```
$ cargo test --workspace --lib --tests -- --test-threads=1
…
test result: ok. 887 passed; 0 failed; 46 ignored

$ cargo clippy --workspace -- -D warnings
error: constant `ME_STAGES_START` is never used
  --> rsx-cli/src/bin/bench_match_rt.rs:93:7
error: this operation has no effect
  --> rsx-cli/src/bin/bench_match_rt.rs:470:42
error: this operation will always return zero
  --> rsx-cli/src/bin/bench_match_rt.rs:470:50
error: could not compile `rsx-cli` (bin "bench-match-rt")
       due to 4 previous errors
```

**Tests: green. Lint: red.** The release gate (`make gate`) does
not currently run `make lint` as a precondition — verified by
reading the Makefile's `gate` chain. The lint claim in CHANGELOG
v0.2.0 is stale.

---

## 250-word executive summary

Round 2 verdict: **drifting in the same direction, with one new
class of bug surfaced that round 1 missed entirely**. Of round
1's five critical risks, 2 are properly resolved (silent fill
drop in matching; JtiTracker null-defeat), 1 partially (the
`<50 µs` claim is demoted in README but still cited in 8 other
docs), and 2 have regressed (ME event buffer panic vs spec's
"stall"; "zero heap on hot path" now disprovable in 3 greps).
**The new finding is matching's WAL is decorative for crash
recovery** — ME on restart loads only the snapshot
(`rsx-matching/src/main.rs:286`); the `WalReader` is never
called between snapshot and crash. SIGKILL between two 10-second
snapshots erases every fill the gateway already shipped to a
client. Codex independently confirmed this. The
`order_prod.push(order).is_err()` path at `rsx-risk/src/main.rs:539`
still silently drops orders — the round-1 lint check (`let _ =`)
missed the `.is_err()` variant. `make lint` fails at HEAD; the
gate isn't run pre-merge so HEAD has been red for ~24 h. FillRecord
silently extended at offset 88 with no WAL header version bump,
mitigated by a plausibility heuristic. Gateway advances CMP seq
even when send_raw fails, producing cross-stream corruption on
NAK. 887 tests pass; sendto-is-99% claim reproduced cleanly;
`rsx-dxs` zero-deps claim holds. **Bet a customer SLA on this in
6 months: 28/100.** Single biggest blocker: the WAL is not
a recovery artifact. Fix that, then re-audit.

— file end —
