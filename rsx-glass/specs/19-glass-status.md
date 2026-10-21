---
status: reference
---

# 19 — What glass has and what it lacks

This file measures glass against the terminal it replaces: for every
capability in specs 02-14, whether `rsx-glass` at the pin has it, has part of
it, or has none of it, with the file that holds it. Glass at the pin is a
read-only heatmap viewer plus a read-only accounts table; its v1 scope
excludes input and trading [doc rsx-glass/SPEC.md "Out of scope (v1)"] — a
scope the founder's decision of 2026-10-21 lifts [founder F-SURF-04, F-SURF-05].

Source pin: `rsx-glass` at `5d175b6ff04755e02fcca5ebbb918b01b929c23b`
(56 tracked files).

Legend: **has** · **part** (what is missing follows) · **none**.

## Foundation

| Capability | Glass | Where |
|---|---|---|
| model reuse (book, heatmap, fisheye, cascade, persistence, basis, session opens, tape, position, latency window, seq, wire decode and order fold) | has, through `rsx-core` | `go.mod` `replace rsx-core => ../rsx-core` |
| movable render seam (`Renderer`: Pump / Present / Close), newest-grid loop, alloc-free present | has | `render.go`, `glass.go`, `soft/soft_test.go::TestPresentAllocFree` |
| software renderer (reference, goldens, headless) | has | `soft/` |
| OpenGL backend (instanced quads, atlas texture, vsync-on, resizable letterbox, parity with soft ≤ 1 level) | has | `gpu/`, `gpu/parity_test.go::TestGPUSoftParity` |
| glyph atlas (Go Mono ASCII + primitive heat glyphs) | has | `atlas/atlas.go` |
| palette | part: Page, Live, Ask, Heading, Text, TextBright, Muted, Degraded; lacks Accent, Ring, PanelBg, Border and the derived / armed styles | `grid.go` |
| colourblind theme | has | `grid.go` `UseTheme` |
| contrast matrix | part: glyph inks and trade overlays (`heat_test.go::TestGlyphContrast`); lacks text-ink and accent rows | `heat_test.go` |
| bold / italic text | none (the atlas has one face) | `atlas/atlas.go` |
| window-derived grid size | none: fixed 72 × 24 letterboxed (OD-23) | `cmd/glass/main.go:43-48` |
| Mailbox present mode, 240 Hz | none (vsync-on; OD-21) | `rsx-glass/SPEC.md` |

## Screens

| Screen / element | Glass | Missing |
|---|---|---|
| BOOK heatmap body (ramp, count, persistence, per-side trade ramps, overlay cap, NOW bars, reserved inks) | has | far-row treatment (OD-02) |
| BOOK gutter + session-open labels | has | — |
| BOOK ruler | part: baseline and touch ticks | own-order marks, cursor |
| BOOK header | part: badge, link dot (md link), mid | axis-centre vs live-mid labelling (BK-02), order link (BK-03) |
| BOOK footer | part: ask / bid touch ladders (raw integers, `x` separator), `~mark` | display precision (raw px/qty at the pin), position line, latency line, hint line, status line |
| BOOK news rail, tape rail, own orders, cursor, microscope | none | all |
| mode line | none | all (CH-01) |
| order-entry view (ladder, form, confirm, positions, risk row, orders, trades, speed strip, trace HUD, ARMED) | none | all |
| NEWS (sector map, co-move, feed, search) | none | all |
| LLM (context block, placeholder, live transcript, input) | none | all |
| USERS | part: a read-only accounts table (id, login, role, collateral, equity, uPnL; empty state; fixture rows); goldens `accounts.png`, `accounts_empty.png` | list → detail → prompts, search, role header, positions/fills/funding/freezes, every write |
| degraded states | part: `degraded.png` (link down) | md stale, crossed book, sizing, empty sides |
| help overlay, hint line, keycast, symbol switcher, venue picker | none | all |
| key legend, chat key, tooltips | none | all (founder requests) |

## Input and order path

| Capability | Glass | Where |
|---|---|---|
| keys | part: `Esc`/`q` quit | `gpu/gpu.go` |
| the one keymap table, rebinds, help generation | none | — |
| order socket (JWT, `N`/`C`, heartbeat echo, reconnect, fold) | none | — |
| fat-finger guards, read-only venue block | none | — |
| per-hop latency legs | none | — |

## Connectors and data

| Capability | Glass | Where |
|---|---|---|
| RSX market-data connector (subscribe, decode, heartbeat echo, reconnect, malformed-frame skip) | has (one symbol) | `live/live.go`, `live/integration_test.go::TestLiveReconnect`, `::TestLiveMalformedFrameSkipped` |
| multi-symbol subscribe and per-symbol markets | none | — |
| mock exchange (md wire encoder pinned to real bytes, scripted scenario, WS server) | has (md only) | `mockx/`, `mockx/contract_test.go::TestMockEncoderMatchesRealBytes` |
| contract test against a live feed | has (opt-in) | `live/contract_live_test.go::TestContractAgainstLiveFeed` |
| offline order-channel mock | none | — |
| `/v1/symbols` precision | none | — |
| Hyperliquid, Phoenix venues; venue seam | none | — |
| Tree of Alpha news | none | — |
| assistant client (SSE), runner, MCP | none | — |
| `rsx-auth` roles client, support client, fixtures | part: fixture rows only | `accounts.go` |
| session-open config, theme config | has | `cmd/glass/main.go` |

## Tests

Glass tests at the pin: `TestComposeAccounts*` (6), `TestContractAgainstLiveFeed`,
`TestEncodeRoundTrip`, `TestFeed*` (4), `TestGPUSoftParity`, `TestGlyphContrast`,
`TestGolden*` (4), `TestHonestyMarkers`, `TestLiveHeatmapEndToEnd`,
`TestLiveMalformedFrameSkipped`, `TestLiveReconnect`,
`TestMockEncoderMatchesRealBytes`, `TestModelReuseSeam`, `TestPresentAllocFree`,
`TestRealAndMockDecodeIdentically`, `TestTradeSideByShape` (27 functions,
counted at the pin with `grep 'func Test'`). Every acceptance row in
[17-acceptance.md](17-acceptance.md) marked "new", and every terminal test it
names, is work glass does not have yet.

## Documents that point at the terminal

`rsx-glass/README.md`, `rsx-glass/SPEC.md` and `rsx-glass/CLAUDE.md` call
rsx-term "the template" and send readers to `../rsx-term/VISUALS.md`,
`SCREENS.md` and `notes/element-inventory.md`. After the deletion those links
break; README and SPEC point at [index.md](index.md) (this change), and
`rsx-glass/CLAUDE.md` needs the same re-pointing (outside this change's
scope). Repo-level references to rsx-term (root `CLAUDE.md`, `README.md`,
`ARCHITECTURE.md`, `specs/2/55-terminal.md`, `specs/2/60-terminal-assistant.md`,
`specs/2/1-architecture.md`, `Makefile` targets) are listed in
[20-rsx-term-record.md](20-rsx-term-record.md).
