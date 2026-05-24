# CMP reliability fixes (deferred)

Three bugs in `rsx-dxs/src/cmp.rs` discovered via narrative
review. All affect behavior under packet loss. Specced here
so we can come back later; **do not implement in this sprint**
(blocks the reliability bench, which itself is deferred).

Status: spec only. Implementation deferred to a future
sprint. Reliability bench (next section) cannot honestly
run until at least bug #1 is fixed.

---

## Bug 1: silent reorder_buf overflow

### Today

`cmp.rs:706-720`:

```rust
} else {
    warn!("reorder buf full ({}), skip gap {}..{}", ...);
    self.reorder_buf.clear();
    self.expected_seq = seq + 1;
    return Some((hdr, payload.to_vec()));
}
```

When `reorder_buf` reaches its 512-slot default limit, the
receiver clears the buffer, advances `expected_seq` past
the gap, and delivers the current packet. The lost seqs
are **silently dropped** at the protocol level. Only a
`warn!` log records the event. Consumer has no signal.

### Why it's broken

Violates spec invariant #1 ("Fills precede ORDER_DONE per
order" — `specs/2/6-consistency.md`). The matching engine
or risk shard would process events out of FIFO order
without knowing. Silent corruption.

### Fix shape

Replace silent skip with explicit faulted state surfaced
to the application:

```rust
pub enum CmpRecv {
    Empty,                          // no data right now
    Data(WalHeader, Vec<u8>),       // in-order delivery
    Faulted {                       // unrecoverable gap
        last_delivered_seq: u64,    // app DXS-replays from here+1
        gap_start: u64,
        gap_end_inclusive: u64,
    },
}

pub fn try_recv(&mut self) -> CmpRecv { ... }

pub fn reset_after_replay(&mut self, new_tip: u64) {
    self.expected_seq = new_tip + 1;
    self.reorder_buf.clear();
    self.faulted = false;
}
```

Once `Faulted` is returned, the receiver becomes sticky:
returns only `Faulted` or `Empty` until `reset_after_replay`
is called.

### Consumer side

Three consumers need to handle the fault: rsx-risk,
rsx-marketdata, rsx-mark. Pattern:

```rust
match cmp_receiver.try_recv() {
    CmpRecv::Data(hdr, payload) => process(hdr, payload),
    CmpRecv::Empty => continue,
    CmpRecv::Faulted { last_delivered_seq, .. } => {
        error!("CMP faulted, replaying via DXS");
        let mut dxs = DxsConsumer::new(stream_id, addr, tip_file, None)?;
        dxs.run_once(|rec| { process(rec.header, rec.payload); true }).await?;
        cmp_receiver.reset_after_replay(dxs.tip);
    }
}
```

The DXS/TCP path already exists at `rsx-dxs/src/client.rs`.

### Tests

- Unit test: fill reorder_buf to limit + 1 packet, assert
  `Faulted` returned and stays sticky.
- Unit test: `reset_after_replay` clears state, resumes
  normal delivery.
- Integration test (existing harness in `rsx-marketdata/
  tests/replay_e2e_test.rs` may already cover the
  replay-after-fault shape; verify or extend).

### Spec amendment

`specs/2/4-cmp.md` should document:
- Reorder buffer is bounded (config: `reorder_buf_limit`,
  default 512).
- On overflow → receiver enters FAULTED state. Within-stream
  delivery stops. Application must replay via DXS/TCP from
  `last_delivered_seq + 1`, then call `reset_after_replay`.
- FIFO contract is preserved: there is no third path where
  the protocol silently advances.

LOC estimate: ~30 LOC in `cmp.rs`, ~10 LOC per consumer ×
3, ~50 LOC tests, ~30 LOC spec.

---

## Bug 2: NAK storm on out-of-order arrivals

### Today

`cmp.rs:691-704`:

```rust
} else if self.reorder_buf.len() < self.reorder_buf_limit {
    self.reorder_buf.insert(seq, full);
    self.send_nak(
        self.expected_seq,
        seq - self.expected_seq,
    );
    continue;
}
```

Every out-of-order packet fires a fresh `send_nak` covering
the whole gap `[expected_seq, seq)`. Heartbeat handler at
`cmp.rs:622-633` ALSO re-fires NAK unconditionally every
10 ms if `highest_seen > expected_seq`.

### Why it's broken

If 100 packets arrive out-of-order, sender gets 100 NAKs
with overlapping ranges. Each NAK retransmits every seq
in its range → O(N²) retransmit traffic. Under sustained
loss this is congestive collapse.

For comparison:
- TCP fast retransmit: 3 dup-ACKs → one retransmit. SACK
  tracks holes; subsequent dup-ACKs don't re-trigger.
- Aeron: NAK rate-limited per gap (`nak.delay`, default
  60 µs unicast).
- MoldUDP64: receiver tracks "request sent" flag per gap.
- QUIC: range-based ACKs, each lost packet retransmitted
  at most once per RTT.

### Fix shape

Per-gap debounce in `CmpReceiver`:

```rust
struct GapTracker {
    last_nak_ns: u64,
    nak_count: u32,
}

// One entry keyed by gap-start seq (the lowest missing seq
// in a contiguous run).
nak_state: FxHashMap<u64, GapTracker>,
```

Before firing a NAK:
1. Look up `gap_start = expected_seq`.
2. If `now - last_nak_ns < nak_min_interval_ns` (config,
   default 100 µs), skip.
3. Otherwise fire NAK and update tracker.

When gap fills (drain_reorder advances expected_seq past
the entry), remove the tracker entry.

Heartbeat path (`cmp.rs:622-633`) uses the same tracker so
the 10 ms heartbeat doesn't bypass the debounce.

### Config

Add to `CmpConfig`:

```rust
pub nak_min_interval_us: u64,  // default 100
```

100 µs accommodates LAN RTT (~1 µs) with margin. Tunable
for WAN.

### Tests

- Unit test: 100 out-of-order packets arriving in 1 ms →
  ≤ 2 NAKs fired (initial + one debounced).
- Unit test: heartbeat fires after gap-fill cycle →
  fresh NAK sent (not blocked by stale tracker).

### Spec amendment

`specs/2/4-cmp.md` §"NAK semantics" should document:
- One NAK per gap, debounced at `nak_min_interval_us`.
- Heartbeat re-NAKs use the same debounce.
- Receiver tracks per-gap state until the gap fills.

LOC: ~30 LOC `cmp.rs`, ~30 LOC tests.

---

## Bug 3: no sender-side retransmit deduplication

### Today

`cmp.rs:277-310` — `CmpSender::handle_nak`:

```rust
for i in 0..count {
    let seq = nak.from_seq.saturating_add(i);
    // retransmit from ring or WAL, no dedup
}
```

Defensive against the receiver-side bug #2: even if
receiver debounces, duplicate NAKs from the wire (packet
duplication, multipath, etc.) can cause the sender to
re-retransmit recently-sent packets.

### Fix shape

Per-seq last-retransmit-time tracker. Fixed-size ring
(one entry per `SEND_RING_CAPACITY` slot) — already lives
alongside `ring_seqs`/`ring_lens`:

```rust
ring_last_retx_ns: Box<[u64]>,  // parallel to ring_seqs
```

In `handle_nak`, before retransmitting seq:
1. Look up `slot = seq & SEND_RING_MASK`.
2. If `ring_last_retx_ns[slot]` exists and `now - it <
   retx_dedup_window_ns`, skip.
3. Otherwise retransmit and update.

Default window: 1 ms (one LAN RTT) — bigger than NAK
debounce so the layers compose.

### Config

```rust
pub retx_dedup_window_us: u64,  // default 1000 (1 ms)
```

### Tests

- Unit test: receive 5 NAKs for the same seq within 100 µs
  → 1 retransmit fired.
- Unit test: receive same NAK twice with 2 ms between →
  2 retransmits (window expired).

LOC: ~30 LOC `cmp.rs`, ~30 LOC tests.

---

## Order of work

These three should land together as one logical unit; the
reliability bench depends on all of them. Suggested commit
sequence:

1. `[dxs] cmp: FAULTED state + CmpRecv enum + reset_after_replay`
2. `[dxs] consumers: handle CmpRecv::Faulted via DXS replay`
3. `[dxs] cmp: per-gap NAK debounce`
4. `[dxs] cmp: per-seq retransmit dedup`
5. `[spec] cmp: document FAULTED, NAK debounce, retx dedup`

Total LOC: ~200 src + ~150 tests + ~80 spec ≈ 1-2 day
focused sprint when picked up.

---

## Reliability bench (downstream)

Once 1-3 land, the reliability harness becomes meaningful.

### Harness shape

- `scripts/bench_reliability/netns_setup.sh` — creates
  `ns_a`, `ns_b`, veth pair, applies tc netem.
- `rsx-dxs/benches/reliability_harness/` — single binary
  with `--protocol {cmp,tcp,quinn,aeron} --impairment NAME
  --rate-pps N --duration-s N`. Writes JSONL.
- `scripts/bench_reliability/post_process.py` — reads
  JSONL, emits markdown summary.

### Impairment matrix

| name | tc spec |
|---|---|
| clean | none |
| uniform-0.1% | `netem loss 0.1%` |
| uniform-1% | `netem loss 1%` |
| uniform-5% | `netem loss 5%` |
| burst-50 | `netem loss 50%` for 5 ms then resume |
| reorder-5% | `netem reorder 5% 50%` |
| latency-spike | `netem delay 0ms 50ms` for 100 ms |
| asymmetric | outbound 0.1%, inbound 1% |
| partition-1s | drop all for 1 s |
| partition-30s | drop all for 30 s |

4 protocols × 10 impairments × 3 rates ≈ 1 h bench time.

### Output: `facts/reliability-benchmarks.md`

Per-protocol table:

| protocol | clean p50 | 1% loss p50 | 1% loss p99 | silent loss at 5%? | recovery from 30s partition |
|---|---:|---:|---:|---|---|
| CMP (pre-fix) | 11 µs | tbd | tbd | **yes** ← bug 1 | tbd |
| CMP (post-fix) | 11 µs | tbd | tbd | no | replay via DXS |
| TCP | 14 µs | tbd | ~200 ms (RTO) | no | hangs at app |
| Aeron | 21 µs | tbd | tbd | no (image disconnect) | replay via Archive |
| Quinn | 37 µs | tbd | tbd | no (stream closes) | reconnect required |

### Cross-check methodology

Sender records (sent_seq, sent_ts). Receiver records
(recv_seq, recv_ts, sender_ts). Post-process compares the
two — never trust a protocol's own "lost" counter; CMP's
silent-skip would underreport.

---

## When to pick this up

Triggers for prioritizing:
- Outside party (auditor, investor) asks "what happens
  under loss?" — need numbers, can't generate them honestly
  without bug 1 fixed.
- Decision to push CMP onto a less-trusted network than
  10 GbE LAN.
- Reliability bench numbers needed for a doc / talk / sale.

Until then: known issues, documented, recoverable in
operational sense (DXS replay always works as a manual
recovery; the bugs hurt automatic recovery, not data
integrity if the operator notices the `warn!` logs).
