---
status: shipped
---

# CMP NAK Retransmit Fix

## Goal

`CmpSender::handle_nak` opens a `WalReader` to retransmit dropped
UDP packets, but the sender never writes to WAL, so `open_from_seq`
always fails and the retransmit is silently dropped. The receiver
(`CmpReceiver`) stalls at the first lost packet, fills its reorder
buffer (512 slots), and starts dropping all subsequent messages.

Fix: add an in-memory send ring to `CmpSender`. On every `send()`,
store `header + payload` bytes indexed by seq. `handle_nak` reads from
the ring instead of WAL. Evict entries older than
`peer_consumption_seq` to bound memory usage.

## File

`rsx-dxs/src/cmp.rs` — single file change.

## Implementation

### 1. Add `send_ring` field to `CmpSender`

```rust
send_ring: std::collections::BTreeMap<u64, Vec<u8>>,
send_ring_limit: usize,   // default 4096
```

Initialise both in `with_config`: `send_ring: BTreeMap::new()`,
`send_ring_limit: 4096`.

### 2. In `send()` — after `send_to` succeeds, store the frame

```rust
// store for potential NAK retransmit
if self.send_ring.len() < self.send_ring_limit {
    self.send_ring.insert(seq, self.buf[..total].to_vec());
}
// evict consumed entries
while let Some(entry) = self.send_ring.first_entry() {
    if *entry.key() < self.peer_consumption_seq {
        entry.remove();
    } else {
        break;
    }
}
```

### 3. In `handle_nak()` — serve from ring instead of `WalReader`

Replace the entire body with:

```rust
pub fn handle_nak(&mut self, nak: &Nak) {
    for i in 0..nak.count {
        let seq = nak.from_seq + i as u64;
        if let Some(frame) = self.send_ring.get(&seq) {
            if let Err(e) = self.socket.send_to(frame, self.dest) {
                warn!("nak retransmit send failed seq={seq}: {e}");
            }
        } else {
            warn!("nak retransmit: seq={seq} not in ring");
        }
    }
}
```

## Acceptance Criteria

1. `cargo build -p rsx-dxs` succeeds with zero errors.
2. `cargo test -p rsx-dxs` passes.
3. Existing `CmpSender`/`CmpReceiver` API is unchanged (no callers
   need to change).
4. `handle_nak` no longer calls `WalReader::open_from_seq`.
5. The "nak retransmit open" warning no longer appears in logs during
   normal operation.

## Constraints

- Do NOT add WAL persistence to `send()` — hot path must stay
  allocation-free after ring is pre-allocated.
- Do NOT change the `CmpConfig` public API unless necessary.
- Keep `send_ring_limit` as a struct field (configurable via
  `CmpConfig` if `CmpConfig` already has an extensible pattern).
- 80 char line width, max 120.
