# CTO Audit — .ship/28-REFINE-AUDIT-2

Adversarial source audit at HEAD `e997598` (master). Read-only.
Targets: Rounds 1-4 changes from .ship/28 and the four open risks
carried in from .ship/27 (CTO #6/#7/#9/#10).

Findings: 5 claims verified; 3 attack scenarios traced; 4 sprint
spot-checks; **grade 68 / 100 — hold (closer to ship than last round)**.
The transport layer is clean. FAULTED recovery infrastructure is wired
but the apply closures are scaffolding — state is NOT re-injected after
a fault for risk, marketdata, or gateway. The "zero malloc on the order
hot path" claim introduced in Round 3 is false at HEAD.

---

## 1. Claim verification

### Claim 1 — "WAL retention 4h via mtime-prune on rotate"
- **Source**: `rsx-cast/src/wal.rs:49` (`RETENTION_NS`),
  `wal.rs:248-286` (`rotate`), `wal.rs:299-352` (`prune_old_segments`).
- **Checked**: `rotate()` at `wal.rs:249` calls
  `prune_old_segments(&self.wal_dir, self.stream_id, RETENTION_NS)`
  after renaming the old active file to a seq-range filename.
  `prune_old_segments` (`wal.rs:302`) reads mtime via
  `entry.metadata()?.modified()`, computes age in nanoseconds, and
  calls `fs::remove_file` on segments older than `RETENTION_NS`.
  The active file is explicitly excluded (`name == active` guard at
  `wal.rs:322-324`). Best-effort: individual `remove_file` failures
  are logged and skipped (`wal.rs:344-349`), not propagated.
- **Verdict**: **CONFIRMED.** This was `wal.rs:81-278` in .ship/27 with
  no implementation. It is now implemented in commit `09d4d87`. The
  previous critical finding is closed.
- **Residual**: Pruning only fires on rotation. A stream with a low
  write rate (< 64 MB per 4 h) may never rotate and will accumulate
  indefinitely. This is documented nowhere; under the "trusted LAN"
  trust model with a running ME this is unlikely to matter in practice,
  but it is a gap between the `RETENTION_NS` constant and the actual
  guarantee.

### Claim 2 — "FAULTED/RECONNECT drain wired for risk, marketdata, gateway" (R1)
- **Source**: `rsx-risk/src/replay.rs`, `rsx-risk/src/main.rs:241-295`,
  `rsx-marketdata/src/main.rs:37-91`, `rsx-gateway/src/main.rs:42-78`.
- **Checked**: All three components have a `drain_replay` function
  (`rsx-risk/src/replay.rs:33-90`, `rsx-marketdata/src/replay.rs:24-79`,
  `rsx-gateway/src/replay.rs:23-78`) with identical signatures.
  `rsx-risk/src/main.rs:596-626` matches all five receivers (gw, me,
  mark, mark-replica, gw-replica) against `CastRecv::Faulted` and
  `CastRecv::Reconnect`. `reset_after_replay(new_tip)` is called
  correctly on each branch (`main.rs:612, 624, etc.`).
- **Critical gap**: The `apply` closure in all three
  `handle_replay` implementations is **log-only**:
  - `rsx-risk/src/main.rs:276-283`: logs `record_type` + `seq`.
    Does NOT re-inject missed orders, fills, or mark updates into
    `RiskShard` state.
  - `rsx-marketdata/src/main.rs:71-78`: same log-only closure.
  - `rsx-gateway/src/main.rs:53-66` (inferred from replay.rs:33-78):
    same.
  The comment at `risk/main.rs:274` explicitly says "Round-1 apply:
  log per-record. State re-injection is a follow-up."
- **Verdict**: **HALF-CONFIRMED.** The infrastructure compiles and
  will no longer panic on FAULTED — that's real progress. But after a
  FAULTED event, risk's in-memory positions, frozen_orders, and account
  balances are missing the gap fills. The system resumes processing new
  orders against **stale risk state**. For a risk tile this is a
  correctness defect: the margin check uses the pre-gap position, so
  a user who received fills during the gap can submit new orders at
  their old (under-utilised) margin and pass a check they should fail.
  This is not a documented known limitation; it is presented as "FAULTED
  wiring complete" in the sprint plan.

### Claim 3 — "Zero malloc on the order hot path" (R3 carry-over)
- **Source**: `README.md:87` (post-2f9ee7d): "Zero malloc on the order
  hot path (GW→Risk→ME→Risk→GW)."
- **Checked**:
  - `rsx-matching/src/wal_integration.rs`: zero allocations confirmed
    (grep returns 0 hits for `Vec::new|to_vec|collect|clone`).
  - `rsx-risk/src/shard.rs:527-628` (`process_order`): calls
    `positions_for_user(order.user_id)` at line 549.
  - `positions_for_user` (`shard.rs:237-256`): returns
    `Vec<&Position>` via `Vec::with_capacity(symbols.len())` at line
    246. **One heap allocation per order check.**
  - The same function is called again from `check_order`
    indirectly via `margin.check_order(..., &positions, ...)`.
- **Verdict**: **FALSE.** The README claim introduced in commit
  `2f9ee7d` is wrong. Risk's `process_order` calls `positions_for_user`
  which allocates a `Vec<&Position>` on every order. The commit message
  says "order hot path is allocation-free" but the code disagrees.
  The CTO #7 fix addressed the _BBO margin scan_ path (removed
  `.to_vec()` in `drain_stashed_bbos`) but did not address the order
  path's `positions_for_user` allocation. CLAUDE.md:32 still carries
  the unqualified "Zero heap on hot path" which is also false.

### Claim 4 — "One CRC per record via prepare → append_framed → send_framed"
- **Source**: `rsx-matching/src/wal_integration.rs:446-462` (`fan_out`),
  `wal_integration.rs:407-440` (BBO carve-out).
- **Checked**: `fan_out` at `wal_integration.rs:448-462` does
  `prepare → append_framed → send_framed [→ optional send_framed]`.
  One `compute_crc32` call in `prepare` (`wal.rs:166`). BBO at
  `wal_integration.rs:434,437` calls `cmp.send(&mut record)` then
  `mkt.send(&mut record)` — two independent `send` calls, each
  computing its own CRC. The comment at lines 413-419 now explicitly
  explains why: BBO bypasses WAL; cmp and mkt are independent streams
  with their own seq counters; `fan_out` cannot be used without a
  shared WAL writer.
- **Verdict**: **CONFIRMED with documented carve-out.** The BBO
  double-CRC is now explained inline, closing CTO #10 cleanly.

### Claim 5 — "Tips monotonic" (correctness invariant #5)
- **Source**: `rsx-cast/src/wal.rs:89` (`next_seq`),
  `wal.rs:161-162` (`prepare`: `seq = self.next_seq; self.next_seq += 1`).
- **Checked**: `WalWriter::prepare` is the only seq-assigning path.
  `next_seq` starts at 1 (`wal.rs:120`) and increments by 1 each call.
  `set_next_seq` (`wal.rs:132-137`) can only raise `next_seq`, never
  lower it (guarded by `self.last_seq < seq.saturating_sub(1)`). Each
  ME symbol has exactly one `WalWriter` (`matching/main.rs:279` —
  only one instantiation per process). No shared-writer scenario.
- **Verdict**: **CONFIRMED.** `next_seq` is strictly monotone per
  stream. No concurrent callers, no wrapping without detection (u64 at
  ~8B events/s wraps in ~73 years).

---

## 2. Three attack scenarios

### Attack A — NAK storm from a LAN peer

**Setup**: Any host on the trusted LAN floods a `CastSender` with
10,000 UDP NAK frames in rapid succession, all requesting the same or
overlapping ranges.

**Trace**:
1. `CastSender::recv_control()` (`cast.rs:476-512`): a non-blocking
   loop that drains all queued UDP packets with `WouldBlock` as the
   exit condition. No iteration cap, no budget limit.
2. Each received RECORD_NAK calls `handle_nak(&nak)` (`cast.rs:497`).
3. `handle_nak` (`cast.rs:370-470`): clamps `nak.count` to
   `SEND_RING_CAPACITY` (4096) at line 374. Per-slot dedup
   (`ring_last_retx_ns`) prevents re-sending the same seq within
   `retx_dedup_window_ns` (default 1 ms).
4. **Bounded but still CPU-intensive**: 10,000 NAK frames × up to 4096
   inner iterations each = up to 40.96M iterations in one
   `recv_control` call. The dedup check is O(1) per slot, but the loop
   still runs. At 1 GHz effective compute, this is ~40 ms of sender-
   thread CPU — which on a dedicated ME core means no orderbook
   processing for 40 ms per NAK storm.
5. **Missing: outer rate limit.** There is no cap on the total number
   of NAKs `recv_control` will process per call. A sustained flood from
   a misbehaving or compromised peer continues consuming CPU
   proportional to flood rate.

**Where it's caught**: Per-slot dedup prevents each slot from being
retransmitted more than once per `retx_dedup_window_ns`. Actual UDP
retransmits are bounded.

**Where it fails**: CPU consumed by the inner loop is unbounded per
call. The spoofed-heartbeat → infinite loop from .ship/27 is **fixed**
(`oldest_missing_run` now clamps to `expected_seq + REORDER_CAPACITY`
at `cast.rs:1116-1118` with a comment citing the CTO audit). But the
sender-side `recv_control` → `handle_nak` path has no similar budget.

**Severity**: medium. Real ME cores are pinned; a 40ms stall on the
ME processing loop is observable as a latency spike. Under the trusted-
LAN model a deliberate flood requires an internal attacker. The fix is
one `nak_budget` counter that caps total `handle_nak` calls per
`recv_control` invocation (e.g. cap at 16).

### Attack B — Crash + partial WAL replay (CRC detection)

**Setup**: ME crashes mid-flush. The active WAL file has a fully written
header (16 bytes, valid version + record_type + len + CRC) but only a
partial payload (e.g. OS buffered a 16-byte header but not the 64-byte
payload before the crash).

**Trace**:
1. `WalReader::next()` (`wal.rs:415-475`) reads 16-byte header via
   `read_exact` (`wal.rs:424`).
2. `WalHeader::from_bytes` validates the header structure. The 16
   bytes fit completely — this succeeds.
3. `let mut payload = vec![0u8; header.len as usize]` (`wal.rs:446`).
4. `file.read_exact(&mut payload)` at `wal.rs:447`: if the payload is
   partially written, `read_exact` returns `UnexpectedEof`.
5. The `UnexpectedEof` branch at `wal.rs:448-457` logs
   "partial record at eof, truncating" and returns `Ok(None)`.
6. ME restarts, replay stops at the partial record. Live writes resume
   from `set_next_seq(last_seq + 1)`.
- **Verdict**: **HANDLED.** Partial write at EOF is correctly truncated.
  No garbage record is passed to the consumer.

**Residual**: CRC mismatch (header CRC valid but payload corrupt) is
also handled: `wal.rs:461-468` returns `Ok(None)` on mismatch. The WAL
reader is robust to crash-truncation and silent bit-flip corruption.
The 10 ms unflushed window remains a documented durability tradeoff.

### Attack C — JWT replay after JtiTracker FIFO eviction

**Setup**: `JTI_TRACKER` is instantiated with `cap=16_384`
(`ws.rs:23-24`). An attacker with access to the auth service generates
16,384 fresh valid tokens (or captures 16,384 distinct tokens from
other users), then submits them to force eviction of a target jti.

**Trace**:
1. `JtiTracker::record()` (`jwt.rs:100-116`): FIFO eviction at
   `order.len() >= self.cap` (`jwt.rs:107`). Evicted jti removed from
   `seen` HashSet. Once evicted, the original jti becomes replayable.
2. The `record` function has **no time-based expiry**. Eviction is
   purely count-based. A jti that was issued with `exp = now + 3600`
   and evicted after 16,384 new connections can be replayed for the
   remainder of its expiry window.
3. `jsonwebtoken::Validation` validates `exp` (`jwt.rs:54-55`). The
   replay attack only works within the token's stated expiry window.
   A 5-minute token (standard for short-lived JWTs) limits the replay
   window to 5 minutes from issue. A 24-hour token (common for dev)
   makes the window 24 hours.
4. `JtiTracker` does not persist across restarts (`jwt.rs:80-85`
   comment: "does not survive restarts"). A gateway restart clears
   all remembered jtis, making any outstanding token immediately
   replayable for the remainder of its expiry.

**Verdict**: The FIFO eviction replay is **known and bounded** by
token expiry. For short-lived tokens (≤ 5 min) the attack window is
narrow. The restart-clears-all behaviour is the more operationally
relevant risk: every gateway restart opens a replay window equal to
the max `exp - now` of any previously-issued token. With 1-hour tokens
(common) that's a 1-hour replay window after every restart.

**Where the spec deferred this**: The `JtiTracker` is explicitly
in-memory FIFO. The spec does not claim post-restart replay protection.
The `rollback` method (`jwt.rs:130-138`) is a net improvement over
.ship/27 — it prevents jti pollution on failed handshakes.

**New finding not in .ship/27**: tokens without a `jti` claim
`return true` unconditionally (`jwt.rs:101-103`). The ws.rs handshake
now requires jti presence (`ws.rs:195-198`, `ok_or("missing jti")`)
before calling `record`. The bypass that Round 1 caught is still
closed — verified at HEAD.

---

## 3. Sprint spot-checks (.ship/28 Rounds 1-4)

### R1 — FAULTED recovery: is it sound or scaffolding?

`rsx-risk/src/replay.rs:33-90` defines `drain_replay`. The function
correctly:
- Creates a single-threaded tokio runtime on demand.
- Pre-seeds `consumer.tip = last_delivered_seq` so the TCP replay
  request starts at the correct seq.
- Calls `reset_after_replay(new_tip)` after `drain_replay` returns
  (`main.rs:612,624`).
- Logs applied/skipped counts.

**What it doesn't do**: the `apply` closure passed from `main.rs:276`
only calls `tracing::debug!(...)`. No record is re-applied to the
shard. A user who had fills during the gap has their position state
stuck at pre-gap values. New orders against that user pass margin
checks against the wrong position.

**Verdict**: The infrastructure is sound. The state-re-injection is
explicitly marked as a follow-up in the code comments. The PLAN.md
calls R1 "highest-ROI item" and the sprint task list marks it
`completed`. The risk correctness gap is real and the comment is
honest, but calling this "completed" is misleading — the panic is gone
but the correctness problem is unresolved.

### R2 — Playground IOC fix in `render_order_trace`

`rsx-playground/pages.py:4368-4375`:
```python
tif = order.get("tif", "GTC")
if tif in ("IOC", "FOK"):
    steps.append(step("amber", "expired", f"{tif} order — no fill received ..."))
else:
    steps.append(step("amber", "open", "resting in book"))
```
**Verdict**: **CORRECT.** Before this fix, unfilled IOC orders showed
as "resting in book" — a falsehood. The fix correctly renders them as
"expired." The `tif` field is read from the stored order dict, which
is set at submission time from the radio button value (`pages.py:2199-
2214`). The data flow is consistent.

### R3 — CTO carry-over: `check_liquidation_for` Vec allocation

The `drain_stashed_bbos` BBO scan path (`shard.rs:843-848`) was fixed:
it now uses index-based access (`users_for_symbol(sid)[i]`) instead
of `.to_vec()`. The two-pass `liq_syms` Vec in `check_liquidation_for`
(`shard.rs:487-491`) was optimised from two loops to one — smaller diff,
same Vec. The funding settlement `Vec<u32>` (`shard.rs:895-898`) keeps
its allocation with a justifying comment.

**What was NOT fixed**: `positions_for_user` (`shard.rs:237-256`)
allocates `Vec<&Position>` on every call. Called from:
- `process_order:549` — **order hot path**
- `check_liquidation_for:476` — BBO margin scan path

The commit message and README.md both claim "Zero malloc on the order
hot path (GW→Risk→ME→Risk→GW)." This is false. `process_order` calls
`positions_for_user` which does `Vec::with_capacity(symbols.len())`
at every order check. The CTO #7 fix addressed the BBO scan path but
left the order path unchanged while updating the documentation to say
the order path is fixed.

**Verdict**: Partial fix, overclaimed documentation.

### R4 — Bench numbers and `make demo-trade`

Commit `e997598` updated WAL bench numbers. The commit message says
"update WAL fsync + random-read bench numbers." This is not verifiable
from source alone (no running the bench), but the numbers are now in
`rsx-cast/ARCHITECTURE.md` with specific bench environment annotations.
Commit `a25f261` adds `make demo-trade` (the `demo-trade` Makefile
target). These are non-controversial ops additions; no logic path to
audit here.

---

## 4. Strengths (do not regress)

1. **Transport core is clean.** `oldest_missing_run` clamp fix
   (`cast.rs:1116-1118`, citing the .ship/27 audit by name) is exactly
   the right fix. Per-slot retransmit dedup in `handle_nak` prevents
   UDP amplification.
2. **WAL retention is now real.** `prune_old_segments` fires on rotate
   with a documented best-effort caveat. The critical finding from
   .ship/27 is closed.
3. **BBO double-CRC is now documented.** `wal_integration.rs:413-419`
   explains exactly why two CRCs are correct for BBO. Auditable.
4. **CRC mismatch + partial EOF detection in WalReader.** Both corrupt-
   record cases (`UnexpectedEof` payload, CRC mismatch) return
   `Ok(None)` — no garbage passed to consumers.
5. **FAULTED no longer panics.** Even with the log-only closures,
   risk/marketdata/gateway will survive a UDP gap without crashing.
   For a demo context this is a real improvement; for production it
   requires completing the state re-injection.
6. **IOC trace fix is correct.** F-N9 from CEO audit is closed cleanly.

---

## 5. Top risks at HEAD

| # | Risk | Citation | Severity |
|---|---|---|---|
| 1 | FAULTED recovery wired but apply-closures are log-only. Risk shard state is wrong after a gap — margin checks run against stale positions. | `risk/main.rs:276-283`, `replay.rs:33-90` | high |
| 2 | `positions_for_user` allocates `Vec<&Position>` per order on the risk hot path. README.md:87 now says "Zero malloc on the order hot path" — this is false. | `shard.rs:237-256`, called from `process_order:549` | medium |
| 3 | `recv_control` has no per-call NAK budget cap. A NAK flood iterates up to 4096 × N_naks inner operations per `recv_control` call, blocking the ME processing loop. | `cast.rs:476-512`, `cast.rs:370-396` | medium |
| 4 | WAL retention only fires on rotation. A low-write-rate stream (< 64 MB / 4 h) never rotates and accumulates indefinitely despite `RETENTION_NS` existing. | `wal.rs:249-284` | low |
| 5 | `try_recv` doc comment at `cast.rs:1061` still references `CastReceiver::poll` — method doesn't exist; real name is `try_recv_with`. Minor but visible to anyone reading the crate's rustdoc. | `cast.rs:1061-1063` | low |
| 6 | README quick-start receiver code (`rsx-cast/README.md:217`) still says "full DXS/TCP cold-start required" — `DXS` was renamed to `replication` in v0.5.0. Stale terminology. | `rsx-cast/README.md:217` | low |
| 7 | JtiTracker restart-clears-all: every gateway restart opens a replay window for all outstanding tokens up to their `exp`. Not documented. | `jwt.rs:80-85` | low (accept-risk candidate) |
| 8 | CLAUDE.md:32 still claims unqualified "Zero heap on hot path" — positions_for_user allocates on every order. | `CLAUDE.md:32`, `shard.rs:246` | low |

---

## 6. Grade

**68 / 100 — hold.**

Why higher than .ship/27 (58):
- WAL retention critical finding (#1 in last report) is genuinely
  closed by commit `09d4d87`.
- Spoofed-heartbeat DoS critical finding (#5 in last report) is
  genuinely closed with a cited fix.
- README API drift bugs (#2, #3 in last report — wrong `new` arity,
  phantom `poll` method) are fixed by commit `622c54f`.
- BBO double-CRC carve-out is now documented.
- FAULTED no longer panics in any of the three consumer components.

Why not higher:
- The single most important sprint deliverable (R1, FAULTED recovery)
  is infrastructure-complete but semantically incomplete. Risk state is
  stale after a gap. The sprint tasks mark it completed but the code
  comments call it a follow-up. A reviewer who reads the commit message
  vs. the apply closure will catch this immediately.
- The "zero malloc on the order hot path" claim introduced in Round 3
  is false at HEAD. The fix addressed the wrong allocation site (BBO
  scan) while the order-path allocation (`positions_for_user`) was left
  in place and the documentation was updated to say it was fixed.
- Two stale terminology/doc items remain in the public-facing crate
  README (`DXS`, `poll`).

**Recommendation**: hold for two targeted fixes before external review:
(a) complete the FAULTED apply-closures for risk at minimum (fills +
order-accepted replay into shard); (b) revert or qualify the "Zero
malloc on the order hot path" README claim, or fix
`positions_for_user` to use a stack-local slice (small-vec approach).
Risks 4-8 are durable but not blocking.
