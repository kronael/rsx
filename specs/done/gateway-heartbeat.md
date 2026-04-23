---
status: shipped
---

# Plan: Server-Initiated Heartbeats + Connection Timeout

## Context

Project: RSX perpetuals exchange (Rust, monoio, WebSocket)
Goal: Gateway sends periodic heartbeats to WS clients and
reaps connections that haven't responded within timeout.

Gateway already echoes client heartbeats. Missing:
server-initiated pings and idle timeout reaping.

---

### Stage 1: Server heartbeat + idle timeout

**Goal**: Gateway sends heartbeat to each WS client every
N seconds. If client doesn't respond (no messages at all)
within timeout, close connection.
**Files**: rsx-gateway/src/handler.rs, rsx-gateway/src/state.rs
**Subagent**: improve
**Dependencies**: []
**Verification**:
- [ ] cargo check -p rsx-gateway passes
- [ ] cargo test -p rsx-gateway passes
- [ ] Server sends periodic heartbeat frames
- [ ] Idle connections reaped after timeout
- [ ] Active connections not affected
