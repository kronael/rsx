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
- ✅ F21-F28 second oracle pass: real affinity, cid-matched
     probe, honest labels (`596d24a`)
- ✅ JtiTracker wire-through ws_handshake (process-wide,
     16K cap, replay rejected with 401) (`72bd481`)
- ✅ BLOG.md reframe + first reliable e2e baseline post-F22
     (p50=11778 µs, p99=347755 µs — the previous capture's
     p99 was undermeasured 7×) (`82f096d`)

All 28 audit findings closed. v0.2.0 carry-over fully cleared.

## Open

(empty — next up is a v0.3.0 release candidate. The natural
v0.3 scope: publish `rsx-dxs` to crates.io with a non-exchange
worked example, draft the SDK packaging that turns the 12
crates into an embeddable "exchange-in-a-box" per WEDGE.md.)

## Backlog

- **10-DEPLOY** — public domain, Docker, TLS, one-click
  reviewer demo.

## Conventions

- Project-level items with concrete acceptance criteria
  graduate to `.ship/NN-NAME/` via `/ship` skill.
- Per-session multi-step tracking uses `TaskCreate`, not
  this file.
- Architectural design questions go to `specs/`.
