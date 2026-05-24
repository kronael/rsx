# CMP reliability v4 — ring reorder + oldest-run NAK + FAULTED

Supersedes `.ship/23-CMP-RELIABILITY-FIXES/SPEC.md` and
`.ship/25-CMP-RELIABILITY-V2/SPEC.md`.

## Status

Specced, awaiting implementation. Targets crate v0.3.0 (current: 0.2.0).
No `WalHeader.version` bump — wire format unchanged, only behavior
(reorder ring, FAULTED state, NAK debounce, sender retransmit dedup).

Sign-off received 2026-05-24:
- Ring-buffer reorder (mirrors `send_ring` on sender)
- Oldest contiguous missing run NAK (no waste)
- FAULTED escalation on slot conflict
- Range NAK kept (no bitmap — simpler, can be extended later)

Codex critique on the prior v2 design drove this simplification. The
v4 design preserves "no silent data loss + bounded NAK rate +
escalation to TCP replay" while cutting ~50% of the proposed complexity.

## What's wrong today

1. **Silent reorder_buf overflow.** `CmpReceiver` advances past lost
   seqs after 512 buffered out-of-order packets, only logging a `warn!`.
   Violates spec invariant #1 (FIFO per stream). Cited in
   `cmp.rs:706-720`.
2. **NAK storm.** Every out-of-order arrival fires a fresh NAK over
   the whole gap range. Heartbeat handler re-fires unconditionally
   every 10 ms. O(N²) retransmit traffic under sustained loss.
3. **No sender-side retransmit dedup.** Duplicate NAKs cause the
   same seq retransmitted N times.

## v4 design

### Data structure: ring-buffer reorder

Replace the current `BTreeMap<u64, Vec<u8>>` with a fixed-size ring
buffer mirroring the sender's `send_ring`:

```rust
const REORDER_CAPACITY: usize = 2048;             // power of 2
const REORDER_MASK: u64 = (REORDER_CAPACITY - 1) as u64;
const REORDER_FRAME_BYTES: usize = 128;           // == SEND_RING_FRAME_BYTES

struct CmpReceiver {
    // ... existing fields (socket, sender_addr, last_drop_warn, etc.)
    expected_seq: u64,
    highest_seen: u64,

    // Reorder ring — replaces reorder_buf BTreeMap.
    // Empty slot iff reorder_seqs[i] == 0.
    reorder_seqs:   Box<[u64]>,   // [REORDER_CAPACITY]
    reorder_lens:   Box<[u16]>,   // [REORDER_CAPACITY]
    reorder_frames: Box<[u8]>,    // [REORDER_CAPACITY * REORDER_FRAME_BYTES]

    // NAK rate-limit state. Only the *oldest* gap matters because
    // every later one is gated behind it (FIFO delivery).
    last_nak_at_ns: u64,
    nak_retries_on_oldest: u16,

    // FAULTED is sticky. Cleared via reset_after_replay().
    faulted: bool,

    buf: [u8; PACKET_BUF_SIZE],
}
```

Memory: 2048 × (8 + 2 + 128) = 283 KB per receiver. Pre-allocated at
construction. **Zero heap allocations on the hot path**, matching the
sender's discipline.

### Constants

| const | value | rationale |
|---|---:|---|
| `REORDER_CAPACITY` | 2048 | At 10 k pps: 200 ms of burst tolerance. At 1 k pps: 2 s. Comfortable margin above realistic LAN hiccups. |
| `REORDER_FRAME_BYTES` | 128 | Same as `SEND_RING_FRAME_BYTES`. All current CMP records ≤ 64 B payload + 16 B header. |
| `nak_retry_us` | 100 | LAN RTT is ~µs; 100 µs gives sender time to retransmit + arrive. Per codex's critique: 1 ms is two orders of magnitude too slow. |
| `MAX_NAK_RETRIES` | 8 | Per-oldest-gap retries before FAULTED. 8 × 100 µs = 800 µs total recovery budget before TCP replay kicks in. |

### Receiver-side API change

Replace `Option<(WalHeader, Vec<u8>)>` return with explicit enum:

```rust
pub enum CmpRecv {
    Empty,                          // no data right now
    Data(WalHeader, Vec<u8>),       // in-order delivery
    Faulted {                       // unrecoverable gap
        last_delivered_seq: u64,
        gap_start: u64,
        gap_end_inclusive: u64,
    },
}

pub fn try_recv(&mut self) -> CmpRecv { /* ... */ }
pub fn reset_after_replay(&mut self, new_tip: u64) { /* ... */ }
```

Once `Faulted` is returned, the receiver becomes sticky — only
returns `Faulted` or `Empty` until `reset_after_replay` is called.

### Receiver logic

#### Insert (out-of-order packet arrives)

```rust
fn reorder_insert(&mut self, seq: u64, payload: &[u8]) -> Result<(), Fault> {
    if payload.len() > REORDER_FRAME_BYTES {
        return Err(Fault);  // payload too big for slot — protocol bug
    }
    let slot = (seq & REORDER_MASK) as usize;
    match self.reorder_seqs[slot] {
        0 => { /* empty — proceed */ }
        existing if existing == seq => { return Ok(()); }  // dup, ignore
        _ => {
            // Slot conflict: ring has wrapped past an unfilled slot.
            // Gap exceeds REORDER_CAPACITY → unrecoverable in-band.
            return Err(Fault);
        }
    }
    self.reorder_seqs[slot] = seq;
    self.reorder_lens[slot] = payload.len() as u16;
    let off = slot * REORDER_FRAME_BYTES;
    self.reorder_frames[off..off + payload.len()]
        .copy_from_slice(payload);
    Ok(())
}
```

#### Drain (advance expected_seq past contiguous buffered packets)

```rust
fn drain_reorder(&mut self) -> Option<(WalHeader, Vec<u8>)> {
    let slot = (self.expected_seq & REORDER_MASK) as usize;
    if self.reorder_seqs[slot] != self.expected_seq {
        return None;
    }
    let len = self.reorder_lens[slot] as usize;
    let off = slot * REORDER_FRAME_BYTES;
    // (assumes header is in the framed bytes; adjust if payload-only)
    let bytes = self.reorder_frames[off..off + len].to_vec();
    let hdr = WalHeader::from_bytes(&bytes[..WalHeader::SIZE])?;
    let payload = bytes[WalHeader::SIZE..].to_vec();

    self.reorder_seqs[slot] = 0;  // clear the slot
    self.expected_seq += 1;
    self.nak_retries_on_oldest = 0;  // progress! reset retry budget
    Some((hdr, payload))
}
```

(In practice `drain_reorder` is called in a loop until it returns None.)

#### Oldest missing run (for NAK)

```rust
fn oldest_missing_run(&self) -> Option<(u64, u64)> {
    if self.expected_seq >= self.highest_seen { return None; }
    let from = self.expected_seq;
    let mut seq = from;
    while seq <= self.highest_seen {
        let slot = (seq & REORDER_MASK) as usize;
        if self.reorder_seqs[slot] == seq { break; }
        seq += 1;
    }
    Some((from, seq - from))  // (from_seq, count)
}
```

Worst-case walk is `REORDER_CAPACITY = 2048` array reads (~2 µs); the
typical case (one missing seq) is one array read.

#### NAK rate-limit + retry (inlined into try_recv)

```rust
fn maybe_nak(&mut self, now_ns: u64) {
    if now_ns - self.last_nak_at_ns < (self.nak_retry_us * 1000) {
        return;  // debounced
    }
    if let Some((from, count)) = self.oldest_missing_run() {
        self.send_nak(from, count);
        self.last_nak_at_ns = now_ns;
        self.nak_retries_on_oldest += 1;
        if self.nak_retries_on_oldest > MAX_NAK_RETRIES {
            self.fault();  // 8 retries without progress on oldest gap
        }
    }
}

fn fault(&mut self) {
    self.faulted = true;
    // try_recv will now return Faulted until reset_after_replay
}
```

Called once per `try_recv` invocation (with `Instant::now()` checked
inline). No app-level "tick" required — runs whenever the consumer
polls.

#### Heartbeat path

Same `maybe_nak()` mechanism. The heartbeat handler updates
`highest_seen` and calls `maybe_nak()`; the debounce ensures heartbeat
NAKs are rate-limited identically to data-path NAKs.

### Sender-side changes

#### Retransmit dedup (defensive)

```rust
struct CmpSender {
    // ... existing fields including send_ring
    ring_last_retx_ns: Box<[u64]>,  // parallel to ring_seqs
}

fn handle_nak(&mut self, nak: &Nak) {
    let now_ns = self.now_ns();
    let count = nak.count.min(SEND_RING_CAPACITY as u64);
    if count != nak.count {
        warn!("nak count={} clamped to {}", nak.count, count);
        // Receiver's next retry naturally re-NAKs the unclamped portion;
        // no special handling needed here.
    }
    for i in 0..count {
        let seq = nak.from_seq.saturating_add(i);
        let slot = (seq & SEND_RING_MASK) as usize;
        if self.ring_last_retx_ns[slot] != 0
            && now_ns - self.ring_last_retx_ns[slot] < (RETX_DEDUP_WINDOW_US * 1000) {
            continue;  // already retransmitted recently
        }
        // ... existing retransmit-from-ring-or-WAL logic
        self.ring_last_retx_ns[slot] = now_ns;
    }
}
```

Constant: `RETX_DEDUP_WINDOW_US = 1000` (1 ms). Bigger than NAK
retry interval so the layers compose: receiver waits 100 µs between
retries, sender ignores duplicate NAKs that race within 1 ms.

#### NAK clamp at 4096 — keep, document

The existing `SEND_RING_CAPACITY` clamp in `handle_nak` stays. For
gaps > 4096 the receiver's next NAK retry will request the remainder;
total recovery happens in `ceil(gap / 4096)` rounds. For gaps that
size, FAULTED + DXS is usually the better answer anyway.

### Consumer-side pattern

The three consumers (`rsx-risk`, `rsx-marketdata`, `rsx-mark`) handle
`CmpRecv::Faulted` by switching to DXS/TCP replay:

```rust
loop {
    match cmp_receiver.try_recv() {
        CmpRecv::Data(hdr, payload) => process(hdr, payload),
        CmpRecv::Empty => yield_or_continue(),
        CmpRecv::Faulted { last_delivered_seq, .. } => {
            error!("CMP faulted, replaying via DXS");
            let mut dxs = DxsConsumer::new(stream_id, server_addr, tip_file, None)?;
            dxs.run_once(|rec| { process(rec.header, rec.payload); true }).await?;
            cmp_receiver.reset_after_replay(dxs.tip);
        }
    }
}
```

### Wire format

**No changes.** Existing `Nak { from_seq, count, _pad1[48] }` stays.
Bitmap NAK explicitly deferred — can be added as a future
backwards-compatible extension if measurement shows scattered loss
recovery is a real bottleneck.

### What's deleted from current code

- `BTreeMap<u64, Vec<u8>>` reorder_buf and its 512-entry limit check
- `peer_consumption_seq`, `peer_window`, `handle_status` — already
  deleted in 87b223e
- The silent-skip path in `try_recv` at `cmp.rs:706-720`
- The unconditional NAK-on-every-out-of-order-arrival behavior

### What's added

- Three parallel ring arrays (`reorder_seqs`/`lens`/`frames`)
- `CmpRecv` enum + `reset_after_replay` API
- `oldest_missing_run` + `maybe_nak` debounce logic
- `ring_last_retx_ns` on sender for dedup

LOC estimate:
- `cmp.rs`: ~80 new receiver, ~20 new sender, ~30 removed → +70 net
- Three consumer integrations: ~15 LOC each × 3 = 45
- Tests: ~150
- Spec amendments to `specs/2/4-cmp.md`: ~50

Total: **~315 LOC** across the workspace.

## Implementation order

1. Add `RetxDedupWindow` + `ring_last_retx_ns` to `CmpSender`. Smallest defensible unit. Lands first.
2. Define `CmpRecv` enum and `reset_after_replay` API on `CmpReceiver`. No behavioral change yet.
3. Add the reorder ring arrays. Keep `BTreeMap` running in parallel for one PR to allow A/B testing if desired; remove after.
4. Wire `oldest_missing_run` + `maybe_nak` + `nak_retries_on_oldest`. Replace existing NAK-on-arrival logic.
5. Implement FAULTED state transition (slot conflict on insert).
6. Update three consumers to handle `CmpRecv::Faulted` → DXS replay.
7. Tests:
   - Single packet loss → NAK fires → retransmit arrives → drain → delivery.
   - Multiple gaps → oldest-run NAKed first → sequential recovery.
   - Slot conflict → FAULTED returned and sticky.
   - `reset_after_replay` resumes normal delivery.
   - Sender-side dedup: 5 NAKs for same seq within 1 ms → 1 retransmit.
8. Integration test with `tc qdisc netem loss 1%` (manual, requires root).
9. Spec amendment in `specs/2/4-cmp.md`.

## Tests to write

```rust
// 1. single packet loss + NAK recovery
#[test]
fn nak_recovers_single_packet() { ... }

// 2. multiple gaps; oldest-run-first NAK pattern
#[test]
fn oldest_missing_run_naks_sequentially() { ... }

// 3. FAULTED on slot conflict
#[test]
fn ring_overflow_faults() { ... }

// 4. FAULTED sticky until reset
#[test]
fn faulted_state_blocks_further_recv() { ... }

// 5. reset_after_replay resumes
#[test]
fn reset_after_replay_clears_fault() { ... }

// 6. sender-side dedup
#[test]
fn handle_nak_dedups_within_window() { ... }

// 7. heartbeat-driven gap detection (idle stream)
#[test]
fn heartbeat_triggers_nak_on_idle_gap() { ... }

// 8. progress resets retry counter
#[test]
fn drain_reorder_resets_nak_retries() { ... }
```

## Spec amendment to `specs/2/4-cmp.md`

Add a "Reorder buffer + FAULTED escalation" section that:
- Specifies `REORDER_CAPACITY = 2048` and its consequence ("max
  in-flight gap window before FAULTED").
- Documents the three failure tiers: NAK (in-band recovery) → FAULTED
  (out-of-band recovery via DXS/TCP replay).
- States the FIFO contract: "Within a stream, CMP delivers in strict
  seq order. No silent skip path — overflow forces FAULTED, never
  out-of-order delivery to the application."
- Removes the stale "in-memory ring only" claim at lines 364-365 and
  384-386 (independently flagged by oracle on the docs review).

## Open follow-ups (not blocking)

1. **Bitmap NAK as future extension.** If reliability bench (when
   eventually built) shows scattered loss is a measurable hot path,
   add a `RECORD_NAK_BITMAP = 0x14` record carrying 448-bit bitmap.
   Backwards-compatible — sender can choose which to emit based on
   gap density.

2. **Reorder ring sizing.** 2048 is a baseline. Per-stream tuning
   may be desirable (mark stream is bursty; matching stream is steady).
   Add `reorder_capacity: usize` to `CmpConfig` if measurements
   support per-stream variation.

3. **Reliability bench harness.** Once v4 lands, build the
   netns+netem matrix from `.ship/23-CMP-RELIABILITY-FIXES/SPEC.md`
   section "Reliability bench (downstream)". Use it to validate the
   constants (especially `nak_retry_us` and `MAX_NAK_RETRIES`)
   against simulated loss.

4. **`specs/2/4-cmp.md` already-wrong claim about "in-memory ring
   only".** Independent of this spec; should be fixed during spec
   amendment.
