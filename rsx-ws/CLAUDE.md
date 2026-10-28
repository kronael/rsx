# CLAUDE.md — rsx-ws

Local to `rsx-ws/`; the root `../CLAUDE.md` applies. The one WebSocket core of
the two monoio edge daemons. `ARCHITECTURE.md` has the module table and the
request and frame rules.

## Rules

- Domain-free: no JWT, no record type, no dependency on `rsx-types`,
  `rsx-messages` or `rsx-cast`. Auth and message parsing stay in the caller.
- A WebSocket behaviour the gateway and marketdata share is changed here,
  once. NEVER a per-daemon copy of the accept loop, the upgrade read, the
  accept key or a frame guard.
- monoio reactor rules: readiness waits only, no yield-spin, no poll. The
  frame reader is not cancel-safe mid-frame; callers await `readable` first.
- `publish = false`; GPL-3.0-only like the root `LICENSE`.
