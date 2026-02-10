# Critique (Current State)

This critique reflects the repo as it exists now. I did not run tests.

## Top Findings (Ordered by Severity)

### Critical

1) **CMP sequencing is broken end-to-end.**
   `CmpReceiver` expects a sequence in the first 8 bytes of every payload, but
   `CmpSender::send_record` never injects one. Most payload types (e.g. order
   requests, event messages) do not start with a seq field, so the receiver
   treats them as out-of-order and drops them after the first message. Backpressure
   and retransmit logic becomes meaningless.
   - Files: `rsx-dxs/src/cmp.rs`, `rsx-matching/src/wire.rs`, `rsx-risk/src/types.rs`

2) **ME -> Risk payload type mismatch.**
   Matching sends `EventMessage` enums over CMP, but Risk parses only
   `FillEvent` and only when `record_type == RECORD_FILL`. The payload layouts
   do not match, so Risk will misinterpret or drop fills.
   - Files: `rsx-matching/src/main.rs`, `rsx-matching/src/wire.rs`,
     `rsx-risk/src/main.rs`

3) **Orders never reach the matching engine.**
   Risk validates orders and only pushes `OrderResponse` back to Gateway; there
   is no path that forwards accepted orders to ME. Even if CMP were fixed,
   the pipeline still stops at Risk.
   - Files: `rsx-risk/src/shard.rs`, `rsx-risk/src/main.rs`

### High

4) **No external ingress path.**
   Gateway boots a WS listener but never parses protocol frames or forwards
   anything to Risk. The system still has no real external order ingress.
   - Files: `rsx-gateway/src/main.rs`, `rsx-gateway/src/ws.rs`

5) **Marketdata never receives events.**
   Marketdata binds a CMP receiver expecting ME events, but ME only sends CMP
   to Risk. There is no fanout or routing to marketdata.
   - Files: `rsx-matching/src/main.rs`, `rsx-marketdata/src/main.rs`

6) **WAL records never get sequence numbers.**
   `wal_integration` writes records with `seq = 0`; the WAL writer increments
   its own `next_seq` but does not patch payloads. DXS consumers therefore see
   a stream of seq=0 records, which breaks dedup and replay semantics.
   - Files: `rsx-matching/src/wal_integration.rs`, `rsx-dxs/src/wal.rs`,
     `rsx-dxs/src/records.rs`

### Medium

7) **UB risk: unaligned `ptr::read` on UDP payloads.**
   Multiple sites cast `u8` payload buffers to typed structs with `ptr::read`.
   These pointers are not guaranteed to be properly aligned, which is undefined
   behavior. Use `read_unaligned` or `bytemuck`/`zerocopy`.
   - Files: `rsx-dxs/src/cmp.rs`, `rsx-matching/src/main.rs`,
     `rsx-risk/src/main.rs`

## Verified Improvements

- `rsx-risk` now ships a binary with replay + persistence wiring.
- `rsx-recorder` is fully implemented and uses `DxsConsumer`.
- Matching engine uses real nanosecond timestamps and writes DXS-format records.
- Config loading is consistently env-driven across binaries.

## Test Reality

- I did not run tests in this pass.
- `rsx-risk/tests/persist_test.rs` depends on Docker via `testcontainers` and
  will fail in environments without container permissions.

## Documentation Mismatches

- `PROGRESS.md` describes “shipped” gateway/marketdata tiles and an end-to-end
  pipeline, but the runnable mains are still skeletons and core routing is not
  wired.
- Test counts in `PROGRESS.md` likely do not match local runs if Docker-gated
  tests are executed.

## Bottom Line

There is a large amount of logic implemented, but the live data path is still
non-functional. The most urgent fixes are CMP sequencing, payload/layout
compatibility between tiles, and forwarding accepted orders from Risk to ME.
Until those are resolved, the system cannot run end-to-end.
