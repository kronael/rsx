# Plan: Phase 2 — Gateway Wiring

## Context

Project: RSX perpetuals exchange (Rust, monoio, CMP/UDP)
Framework: Rust + monoio + CMP/UDP transport
Goal: Complete gateway wiring — cancel handler, heartbeat,
rate limiting, circuit breaker, auth extraction

**Current state:** Gateway already has working order submission
(NewOrder → CMP → Risk), response routing (fills, done,
cancelled back to user), pending tracking, and WS I/O.
Building blocks exist but aren't wired: rate_limit.rs,
circuit.rs. Cancel is parsed but rejected as "unsupported".

---

### Stage 1: Cancel Order Handler

**Goal**: Implement cancel order flow GW → Risk → ME
**Files**: rsx-gateway/src/handler.rs, rsx-dxs/src/records.rs,
  rsx-gateway/src/pending.rs, rsx-gateway/src/state.rs
**Subagent**: improve
**Dependencies**: []
**Verification**:
- [ ] Cancel by oid (32-char hex) sends CMP cancel to Risk
- [ ] Cancel by cid (20-char) looks up pending, sends cancel
- [ ] Cancel of unknown order returns error
- [ ] cargo check --workspace passes

**Details**:

1. Add `RECORD_CANCEL_REQUEST: u16 = 11` to rsx-dxs/src/records.rs

2. Add `CancelRequest` record to rsx-dxs/src/records.rs:
```rust
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct CancelRequest {
    pub seq: u64,
    pub ts_ns: u64,
    pub user_id: u32,
    pub symbol_id: u32,
    pub order_id_hi: u64,
    pub order_id_lo: u64,
    pub _pad: [u8; 24],
}
```
Implement CmpRecord for it.

3. Add `find_by_client_order_id(&self, cid: &[u8; 20])
   -> Option<&PendingOrder>` to PendingOrders in pending.rs
   (linear scan over queue, return first match).

4. In handler.rs, add `WsFrame::Cancel { key }` match arm:
   - `CancelKey::OrderId(hex)`: convert hex to [u8;16] via
     `hex_to_order_id`, split into hi/lo u64, find in
     pending by order_id, build CancelRequest, send via CMP
   - `CancelKey::ClientOrderId(cid)`: find in pending by cid,
     extract order_id hi/lo, build CancelRequest, send via CMP
   - If not found in pending: send E[1005, "order not found"]
   - On success: send U[oid, 2, 0, 0, 0] (status=2 cancelled,
     optimistic — real cancel confirmation comes from ME)

5. The cancel flows: GW → Risk (passthrough) → ME. Risk
   doesn't need to do anything special for cancels (no margin
   change). ME processes cancel via book.cancel_order().
   For now, just send the CancelRequest to Risk's CMP addr.
   Risk main.rs already has a `_ => {}` catch-all that
   drops unknown record types — that's fine for v1; cancel
   passthrough in Risk is Phase 2b.

---

### Stage 2: Heartbeat Echo + Rate Limiting + Circuit Breaker

**Goal**: Wire existing building blocks into handler
**Files**: rsx-gateway/src/handler.rs, rsx-gateway/src/state.rs
**Subagent**: improve
**Dependencies**: []
**Verification**:
- [ ] H frame echoed with server timestamp
- [ ] Rate limiter rejects when tokens exhausted
- [ ] Circuit breaker checked before sending to CMP
- [ ] cargo check --workspace passes

**Details**:

1. **Heartbeat** in handler.rs: Add `WsFrame::Heartbeat { .. }`
   match arm. Echo back `H[now_ms]` where now_ms is current
   time in ms. Per spec: "Client may also initiate heartbeats;
   server echoes."

2. **Rate limiting**: handler.rs needs access to a per-user
   RateLimiter. Add `user_limiters: FxHashMap<u32, RateLimiter>`
   to GatewayState. In the NewOrder handler, before processing:
   ```rust
   let limiter = st.user_limiters
       .entry(user_id)
       .or_insert_with(|| RateLimiter::per_user());
   if !limiter.try_consume() {
       // send E[1006, "rate limited"]
       continue;
   }
   ```
   Also rate-limit Cancel requests same way.

3. **Circuit breaker**: Add `circuit: CircuitBreaker` to
   GatewayState. Before sending any CMP message in handler.rs,
   check `st.circuit.allow()`. If not allowed, send
   E[5, "overloaded"]. On successful CMP send, call
   `record_success()`. On CMP error, call `record_failure()`.
   Initialize with config values from GatewayConfig.

---

### Stage 3: Auth Extraction from WS Upgrade

**Goal**: Extract user_id from WS upgrade headers instead of
hardcoding 0
**Files**: rsx-gateway/src/ws.rs, rsx-gateway/src/main.rs,
  rsx-gateway/src/handler.rs
**Subagent**: improve
**Dependencies**: []
**Verification**:
- [ ] ws_handshake returns user_id extracted from headers
- [ ] Missing/invalid auth returns HTTP 401
- [ ] handle_connection receives real user_id
- [ ] cargo check --workspace passes

**Details**:

Per WEBPROTO.md: "Auth is via WebSocket upgrade headers only
(JWT in Authorization header)."

For v1, implement simple auth extraction (no JWT validation —
that requires a signing key and crypto dep we don't have yet):

1. In ws.rs, change `ws_handshake` signature to return
   `io::Result<(String, u32)>` — returns (ws_key, user_id).

2. Add `extract_user_id(request: &str) -> Option<u32>` that
   looks for `Authorization: Bearer <token>` header, then
   parses token as a simple u32 (for dev/testing). In
   production this would validate JWT — add a TODO comment.
   If no Authorization header, check `X-User-Id` header as
   fallback (for testing without JWT).

3. In ws_handshake, call extract_user_id. If None, send
   HTTP 401 response and return error.

4. Update main.rs: ws_handshake returns user_id, pass to
   handle_connection instead of hardcoded 0.

5. Update ws_accept_loop signature: handler closure now
   receives (TcpStream, u32) instead of just TcpStream.
   Or better: do handshake inside ws_accept_loop and pass
   user_id to handler. Actually simplest: move handshake
   into handle_connection (it's already there), and extract
   user_id there. Just change ws_handshake return type.

---

### Stage 4: Tests + Cleanup

**Goal**: Add tests for new functionality, update PROGRESS.md
**Files**: rsx-gateway/tests/handler_test.rs (new),
  rsx-gateway/tests/pending_test.rs, PROGRESS.md
**Subagent**: improve
**Dependencies**: [1, 2, 3]
**Verification**:
- [ ] All existing tests pass
- [ ] New cancel handler tests pass
- [ ] New heartbeat test passes
- [ ] New rate limit test passes
- [ ] cargo test --workspace (excluding docker tests)

**Details**:

1. Add to pending_test.rs:
   - test_find_by_client_order_id

2. Add tests/handler_test.rs with unit tests:
   - These should test the protocol-level logic, not full
     async WS. Test the cancel lookup logic, rate limit
     rejection, etc. Since handler.rs is async monoio, test
     the building blocks instead.

3. Add to rsx-dxs/tests/records_test.rs:
   - test_cancel_request_record: verify CancelRequest
     size/alignment, CmpRecord impl

4. Run cargo test --workspace (skip docker-dependent tests)

5. Update PROGRESS.md with new gateway status
