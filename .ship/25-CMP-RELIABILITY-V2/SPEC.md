# CMP reliability v2 — NAK budget + reset

## Source

User design, dictated 2026-05-24, summarized verbatim:

> "there is a nak budget .... 1000 or so... this is nak-in-flight on
> the consumer.  stored in a heap, each turn consumer checks to
> retransmit a nak if the packet is still not there, only one per
> turn the idea is we dont want to overwhealm the network anyway.
> the consumer has a receive buffer which is a heap with say 10k
> entries ... or really huge so it can acommodate temporary network
> outage and recovery.  messages received out-of-order are stored
> temporarily before delivering them to the application, while the
> nak dance continues... .if there is more like MAX_NAK_IN_FLIGHT
> ... or so... say 1k or maybe 100 naks in flight... a reset is
> issued which makes the source re-transmit messages (all of them)
> in sequence .  the buffer of messages is not flushed so this in
> fact makes it just re-transmit many messages to try to re-fill
> all the gaps."

## Algorithm

### Receiver-side state

```rust
struct CmpReceiver {
    // ... existing fields (socket, expected_seq, highest_seen, buf)

    // Out-of-order buffer: indexed by seq.
    // Replaces the current 512-slot BTreeMap; same shape, bigger.
    reorder_buf: BTreeMap<u64, Vec<u8>>,

    // NAK-in-flight tracker. Min-heap by next_retry_ns so the
    // hottest NAK is always at the top. Each entry is one
    // contiguous missing run, NOT one missing seq, so it stays
    // bounded by gap count, not by lost-packet count.
    naks_in_flight: BinaryHeap<NakInFlight>,
}

struct NakInFlight {
    gap_start_seq: u64,         // first missing seq in this run
    gap_end_seq: u64,           // last missing seq (inclusive)
    next_retry_ns: u64,         // when to re-send this NAK
    retries_so_far: u16,        // for stat / debug
}
```

`BinaryHeap` reverse-ordered so `peek()` returns the NAK with the
earliest `next_retry_ns`.

### Constants

| name | value | rationale |
|---|---:|---|
| `REORDER_BUF_LIMIT` | 10 000 | absorbs ~10 s of accumulated out-of-order packets at 1 k pps, or ~1 s at 10 k pps |
| `MAX_NAK_IN_FLIGHT` | 100 | one entry per *gap*, not per *seq*. 100 simultaneous gaps is already very lossy; beyond this, network is broken, force a reset. |
| `NAK_RETRY_INTERVAL_US` | 1 000 (1 ms) | one LAN RTT-plus-jitter. Faster wastes bandwidth without giving sender time to respond. |
| `NAK_PER_TICK_LIMIT` | 1 | "only one per turn" — one NAK sent per receiver iteration. Caps NAK rate at `1 / tick_period`. |

### Receiver loop (one iteration)

```
1. Try to recv a datagram (non-blocking).

2. If a packet was received:
     parse header
     if hb (heartbeat):
         update highest_seen
         (if highest_seen > expected_seq) register_missing_gap(expected_seq..=highest_seen-1)
         return
     if nak / control: handle as before
     else (data record):
         seq = extract_seq(payload)
         if seq == expected_seq:
             deliver(payload)
             expected_seq += 1
             drain_reorder()
             // gap may have just closed → remove from naks_in_flight
             retire_naks_below(expected_seq)
         elif seq > expected_seq:
             // gap detected
             if reorder_buf.len() < REORDER_BUF_LIMIT:
                 reorder_buf.insert(seq, payload)
                 register_missing_gap(expected_seq..=seq-1)
             else:
                 // reorder buf full → unrecoverable via NAK
                 issue_reset()
         elif seq < expected_seq:
             // duplicate or retransmit-of-stale; drop silently
             pass

3. Check naks_in_flight (one NAK per turn):
     if let Some(top) = naks_in_flight.peek():
         if top.next_retry_ns <= now_ns():
             top = pop()
             send_nak(top.gap_start_seq, top.gap_end_seq)
             top.retries_so_far += 1
             top.next_retry_ns = now_ns() + NAK_RETRY_INTERVAL_US * 1000
             push(top)

4. Check overflow → issue_reset() if too many gaps:
     if naks_in_flight.len() > MAX_NAK_IN_FLIGHT:
         issue_reset()
```

### `register_missing_gap(seq_range)`

```
- Coalesce with any adjacent gap already in naks_in_flight
  (so 5..=10 and 11..=20 merge into 5..=20).
- If the merged gap already exists, do nothing (next_retry_ns
  fires it on schedule).
- Otherwise push a new NakInFlight with
  next_retry_ns = now_ns() (fire immediately, but only one
  per turn per the loop above).
```

### `retire_naks_below(seq)`

```
- After expected_seq advances, walk naks_in_flight; for any
  NakInFlight whose gap_end_seq < seq, remove it.
- For any whose gap_start_seq < seq <= gap_end_seq, trim to
  seq..=gap_end_seq.
```

(Implementation note: a binary heap doesn't support efficient
arbitrary removal. Two options: tombstone-on-peek with lazy
deletion, or use a `BTreeMap<u64, NakInFlight>` keyed by
`gap_start_seq` and re-build the priority view each retry-check.
Tombstones are simpler.)

### `issue_reset()`

Reset is **out-of-band sender-side** — sender restarts at some
seq the receiver tells it. Two variants on the wire:

```
RECORD_RESET_REQUEST  // receiver → sender
{ from_seq: u64,      // sender, please replay from this seq
  to_seq: u64,        // ...up to this seq (current highest_seen)
  _pad: [u8; 48] }
```

Sender on receipt:
- Look up `from_seq..=to_seq` in `send_ring` and WAL.
- Retransmit each in order, as normal data records.
- The receiver, on receipt of each retransmitted packet,
  drops it into `reorder_buf` (or delivers it if `seq ==
  expected_seq`) as usual.

Receiver-side behavior on `issue_reset`:
- Clear `naks_in_flight` (sender will retransmit everything).
- DO NOT clear `reorder_buf` (already-received out-of-order
  packets are still useful).
- Push one new NakInFlight for the full range
  (expected_seq..=highest_seen).
- Send `RECORD_RESET_REQUEST`.

If the reset itself fails (no progress after N reset cycles,
configurable, default 3): escalate to FAULTED state and the
consumer DXS-replays from WAL — same as `.ship/23-CMP-RELIABILITY-FIXES/`.

So we have **three failure tiers**:

| tier | trigger | recovery |
|---|---|---|
| 1. NAK | gap detected | sender retransmits one or more packets from ring/WAL |
| 2. RESET | naks_in_flight > MAX, OR reorder_buf full | sender retransmits the whole window, receiver re-fills gaps |
| 3. FAULTED | N consecutive resets failed | consumer DXS-replays from WAL, then resumes CMP |

### Sender-side state

```rust
struct CmpSender {
    // ... existing fields
    // No new state.  send_ring + WAL fallback already handle
    // arbitrary retransmit ranges.  RESET_REQUEST is just one
    // big NAK range.
}
```

Sender on `RECORD_RESET_REQUEST`: identical to NAK handling
but for a (potentially large) range. The existing
`handle_nak` loop works; just need a new branch in
`recv_control` to parse the new record type.

### Wire format

Add one new record type:

```
RECORD_RESET_REQUEST = 0x14   (next after REPLAY_REQUEST=0x13)

struct ResetRequest {
    from_seq: u64,
    to_seq: u64,
    _pad: [u8; 48],
}
```

64-byte aligned, repr(C), CRC32, same envelope as NAK.

## Constants — full suggested values

| const | suggested | range to consider | reasoning |
|---|---:|---|---|
| `REORDER_BUF_LIMIT` | 10 000 | 1 000 – 100 000 | At 10 k pps (typical exchange rate), 10 000 = 1 s of patience. Heap mem cost: ~2.5 MB at 128 B per entry. Negligible. |
| `MAX_NAK_IN_FLIGHT` | 100 | 100 – 1 000 | User said 100-1000. 100 catches "broken-network" faster (less wasted NAK traffic before reset). 1000 is "more patient." 100 is the right floor — beyond that we're definitely better off resetting. |
| `NAK_RETRY_INTERVAL_US` | 1 000 (1 ms) | 500 – 10 000 | LAN RTT is ~µs; 1 ms gives sender time to retransmit + arrive without bombarding. WAN deployments would want 10-50 ms. |
| `NAK_PER_TICK_LIMIT` | 1 | 1 – 4 | "Only one per turn" per spec. If tick rate is 100 µs, 1/tick = 10 000 NAKs/s max. That's already plenty of bandwidth for recovery. |
| `MAX_RESETS_BEFORE_FAULT` | 3 | 2 – 5 | After 3 full resets fail, the network is unrecoverable in-band; escalate to FAULTED + DXS. |
| `RESET_REQUEST_COOLDOWN_MS` | 100 | 50 – 500 | Don't reset more than once per 100 ms — gives sender time to retransmit the whole window before we ask again. |

## What this design buys us

1. **NAK rate is rigorously bounded**. With "one per turn" and `NAK_RETRY_INTERVAL_US = 1 ms`, NAK traffic per receiver tops out at one packet per millisecond per gap. Across 100 simultaneous gaps that's 100 NAKs/sec, ~6 KB/s — trivial.
2. **Big reorder buffer** (10k vs current 512) absorbs realistic outage bursts. At exchange rates this is ~1 s of breathing room.
3. **Reset escalation** prevents the NAK-storm pathology. Instead of N retries × N gaps = O(N²) wasted bandwidth, we cap at MAX_NAK × retries before forcing a sender-side bulk retransmit (which is much more efficient: sender just walks send_ring sequentially).
4. **No silent data loss anywhere**. The three tiers (NAK / RESET / FAULTED) cascade: each is a less-fancy more-reliable mechanism than the previous. FAULTED ultimately falls back to WAL replay over TCP, which is the strongest possible guarantee.
5. **NAK heap means "one priority queue" of recovery work**, not per-packet bookkeeping. Coalesces adjacent gaps automatically.

## What this design loses

1. **Tail latency under sparse loss**. With `NAK_RETRY_INTERVAL_US = 1 ms`, a single lost packet recovers in ~1-2 ms instead of the current ~10 µs (immediate-NAK-on-arrival). Trade-off for the rate-limiting; on a trusted LAN with sub-ppm loss this is fine.
2. **More state**. NakInFlight heap + retire bookkeeping is ~50 LOC of new logic vs the current naive "fire NAK every gap arrival."
3. **Reset is bandwidth-heavy** when it fires — sender retransmits the whole gap window. Acceptable because reset is rare; the alternative (NAK storm) is worse.

## Implementation order

1. Define `ResetRequest` record + `RECORD_RESET_REQUEST` constant in `protocol.rs`.
2. Extend `CmpReceiver` with `naks_in_flight: BinaryHeap<NakInFlight>` + the helpers (`register_missing_gap`, `retire_naks_below`, `issue_reset`).
3. Replace the existing NAK-on-arrival path with `register_missing_gap`.
4. Add a "NAK pump" step to `try_recv` and/or to `tick` that does one NAK per turn from the heap.
5. Extend `CmpSender::recv_control` to handle `RECORD_RESET_REQUEST` (uses existing `handle_nak` logic with the big range).
6. Wire `REORDER_BUF_LIMIT` to 10k in `CmpConfig`.
7. Unit tests:
   - Single-packet loss → NAK fires → retransmit arrives → reorder_buf drains → delivery resumes.
   - 99 gaps → 99 NAKs in flight, one per ms.
   - 100th gap → RESET fires.
   - 3 consecutive failed resets → FAULTED state.
   - Reset doesn't clear reorder_buf (already-received packets stay).
8. Integration test with `tc netem loss 1%` if available.
9. Spec amendment to `specs/2/4-cmp.md`.

LOC estimate: ~200 receiver, ~30 sender, ~150 tests, ~50 spec = ~430 LOC.

## Open questions for the user before sign-off

1. **`MAX_NAK_IN_FLIGHT = 100 or 1000?`** Trade-off between
   how-many-gaps-we-tolerate-before-reset (higher = more
   resilient to lossy bursts) vs how-quickly-we-give-up-and-bulk-retransmit
   (lower = faster recovery from severe degradation). 100 is the
   right answer if "we expect occasional packet loss; many gaps
   means the network is broken." 1000 is right if "we expect
   occasional bursty multi-packet loss and want to ride it out."
   My recommendation: **100** — exchange-grade trusted LANs
   don't reach 100 simultaneous gaps unless something is
   genuinely broken.

2. **Reorder buffer 10k entries — heap allocation?** At 128 B
   per record × 10k = 1.25 MB. Bounded but heap-allocated.
   Alternative: pre-allocate the full 10k slot array at
   construction (zero-heap path). Trade-off: ~1 MB of memory
   always reserved per receiver. Recommendation: **pre-allocate**
   (matches the rest of CMP's no-heap-on-hot-path discipline).

3. **NAK heap: tombstone-on-peek vs BTreeMap?** Both work,
   tombstones are simpler. Recommendation: **tombstone**.

4. **Reset behavior on sender side**: should the sender
   acknowledge the reset somehow, or just blast retransmits?
   Just blasting is simpler. Acknowledging adds a round-trip
   but lets the receiver know reset was received (useful for
   the "3 failed resets" counter). Recommendation: **don't
   acknowledge** — the receiver counts a reset as "succeeded"
   if the gap eventually fills within `RESET_REQUEST_COOLDOWN_MS`.

5. **Is the FAULTED tier needed at all if we have RESET?**
   Almost certainly yes — RESET can fail (sender unreachable,
   WAL corrupted, network totally partitioned). FAULTED + DXS
   replay is the last line of defense and uses TCP, which is
   strictly more reliable than UDP.

## Next step

You sign off (or push back) on:
- the constants
- the open questions above
- the implementation order
- whether to roll this in alongside or after the
  `.ship/23-CMP-RELIABILITY-FIXES/SPEC.md` work (recommendation:
  **this supersedes #23** — these are the v2 designs that emerged
  from talking it through).

Then a sub implements. Estimate: 1-2 days focused work for code + tests + spec amendment.
