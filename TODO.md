# TODO

Status as of 2026-05-22. v0.2.0 + audit cleanup pass landed.
F21-F28 fix sprint in progress.

Active ship projects live in `.ship/NN-NAME/`. This file is
the light backlog — items not yet a ship project.

## Shipped this round (28 audit findings, agent + manual)

- ✅ F1   CMP `SO_REUSEPORT` + SIGTERM drain (`0120806`)
- ✅ F2-F11   server.py truthful health/verify/WAL/topology (`c190aea`)
- ✅ F12   Trade UI quiet first-paint (`37f6edf`)
- ✅ F13-F19  pulse, hardcoded breaker removed, honest labels (`47b6fce`)
- ✅ F20   `start_all` SIGTERM-first, full-path match (`58b567e`)
- ✅ Playwright regression specs for F3-F11 (`eb30730`)
- ✅ `rsx-book` eprintln → tracing (`bbb0f9f`)
- ✅ `rsx-dxs` 8 MB UDP recv buffer (sustained_throughput stable) (`2fe18e8`)
- ✅ clippy 1.93 gate cleared (`533ccf9`)

## Open

### F21-F28 — second oracle pass (agent in flight)

See `.ship/15-PLAYGROUND-AUDIT/FINDINGS.md` "Oracle pass 2".
The two critical ones for a latency-focused exchange:

- **F21** `/x/core-affinity` invents `Core {i}` from list
  index. Need `os.sched_getaffinity(pid)` or remove the panel.
- **F22** `/api/latency-probe` returns the first frame with
  an `"F"` key without matching the probe `cid`. THE headline
  GW→ME→GW number; can be an unrelated maker fill.

Plus F23-F28 (latency-regression relabel, invariant-status
UNKNOWN-when-empty, ring-pressure relabel, msgs/sec sliding
window, maker/status stale flag, stress/reports surface
corrupt files).

### Carry from v0.2.0

- **JtiTracker wire-through** (`rsx-gateway/src/ws.rs:109`).
  Decision pending: per-process tracker vs shared Redis.
  Per WEDGE.md (B+A: SDK on open-source orthogonal parts),
  per-process is the smaller scope.
- **Measured E2E latency** in `bench-baseline.json`. First
  capture at p50 = 11.7 ms (234× over <50 µs budget) is
  unreliable until F22 lands — the probe was timing unrelated
  fills. Re-capture after F1 + F22 + risk index (`3d151f1`).
- **BLOG.md narrative reframe** per WEDGE.md (B+A: SDK on
  open-source orthogonal parts). Editorial; blocked on
  finishing F21-F28.

## Backlog

- **10-DEPLOY** — public domain, Docker, TLS, one-click
  reviewer demo.

## Conventions

- Project-level items with concrete acceptance criteria
  graduate to `.ship/NN-NAME/` via `/ship` skill.
- Per-session multi-step tracking uses `TaskCreate`, not
  this file.
- Architectural design questions go to `specs/`.
