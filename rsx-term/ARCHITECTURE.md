# rsx-term — Architecture

How the terminal is built internally. For *what it is and how to run it* see
`README.md`; for *why* each design choice was made see `notes/`; for *what each
screen shows* see `SCREENS.md`. This file is the middle layer: the modules, the
data flow, the algorithms, the invariants, and the decisions (with the
alternatives that were rejected).

rsx-term is a keyboard-only, streaming **text Bookmap** — a trading terminal that
renders a live order-book *liquidity heatmap over time* into a character grid,
plus a market-overview screen and an LLM context screen, over a generic
multi-exchange seam.

## Modules

| Package / file | Purpose |
|---|---|
| `wire/` | The normalised wire model every venue maps into: `Snapshot`/`Delta`/`Bbo`/`MdTrade`/`Level` (market data), `OrderReq` + the JSON order/cancel frames, `Folder` (pending→accepted pairing). No exchange-specific types leak past here. |
| `book/` | Pure, clock-free state folds: `Book` (the live order book), `Tape` (trades), `Position` (checked i64 P&L), `Heatmap` (the multi-resolution ring + far-tier cascade + fisheye price mapping), `Persistence` (level age). |
| `conn/` | One file per venue behind the seam: `live.go` (RSX gateway WS), `hyperliquid.go` (HL JSON → normalised, read-only), `mock.go` (scripted offline), `jwt.go` (HS256), `symbols.go` (`/v1/symbols`). |
| `feed/` | The transport-agnostic messages the UI folds — link-state transitions, a latency sample, the `Submitter` interface — decoupling `ui` from `conn`. |
| `news/` | The headline source (`Source`, `Off`, the Tree of Alpha reader) and the UI-agnostic `AssistantContext` handoff to the LLM screen. |
| `ui/` | The Bubble Tea model: event folding, the per-venue `market` fold, the keymap-driven dispatch, and the render (`stream.go` = the heatmap view; `view.go` = the classic DOM view). |
| `main.go` | Env config, venue wiring, the mock/live/venue selection, signal handling. |
| `tools/glyphbank/` | The glyph-calibration utility (see `notes/glyph-bank.md`). |

## Data flow

```
   exchange            conn/ (one Source per venue)        book/ (pure folds)        ui/
 ┌──────────┐  bytes  ┌───────────────────────────┐ wire.* ┌──────────────────┐    ┌─────────┐
 │ RSX WS   │────────▶│ live.go   ─┐               │───────▶│ Book / Tape      │───▶│ model   │
 │ HL WS    │────────▶│ hyperliquid.go ├─normalise─▶│ events │ Position         │    │ (fold)  │
 │ (mock)   │────────▶│ mock.go   ─┘               │        │ Heatmap ring +   │    │         │
 └──────────┘         └───────────────────────────┘        │  far-tier cascade│    └────┬────┘
       ▲   order JSON        ▲  Submitter.Submit/Cancel                                  │ render
       └────────────────────┘                                                            ▼
                                                                              stream.go / view.go
```

Everything left of the `wire.*` boundary is exchange-specific and untrusted;
everything right of it speaks one normalised language. The render is a pure
function of the model, so the model→render boundary is the only thing a future
GPU/bitmap frontend would replace. See `notes/venue-seam.md`.

## The heatmap (the core algorithm)

`book/heatmap.go` is the crown jewel. It answers "show a whole book and its
history in a fixed grid" with **three nonlinear compressions** (full derivation in
`notes/compression.md`):

- **Time — a log cadence.** A short **live ring** of exact ~100 ms rows at the
  bottom (redrawn every frame, so *now* never scrolls away), then **far tiers**
  where each row aggregates an exponentially longer window (10 s, 60 s, 120 s,
  300 s, 600 s, then hours) via a `cascade`: an expiring live row folds into
  tier 0, which seals and promotes into tier 1 when its span fills, and so on.
  Near rows carry the exact book; far rows carry a time-weighted liquidity
  profile of the *whole* book.
- **Price — a fisheye.** `FisheyeCol`/`FisheyePx` map price↔column mid-anchored
  (with hysteresis): 1 tick/cell at the touch, deep levels aggregated by a
  triangular schedule computed in bounded closed form. Rows store **price-space**
  profiles, not screen columns, so a re-anchor re-aligns the whole picture.
- **Size — a log colour ramp** on a stable, slow-decaying basis (no per-frame
  flicker), rendered through the empirically-calibrated glyph vocabulary
  (`notes/glyph-bank.md`): colour = size, glyph = order-count, distinct markers =
  the trade layer.

`book/` is pure and clock-free — the caller supplies bin timestamps — which is
what makes it testable and replay-friendly, and what the BOOK screen's freeze
feature reads from directly.

## Screens

Three screens, one keystroke apart (`tab`/`shift+tab`), all keyboard-only —
there is **no mouse** (a mouse in a fast trading surface is a fat-finger +
latency liability; the price cursor is `h`/`l`):

- **BOOK** — the heatmap on one symbol, for hand market-making; a rapid
  letter-code symbol switcher (`x`+code) hops the watchlist; a keyboard row-cursor
  can **freeze** a row of the visible history and hand it to the LLM screen.
- **NEWS** — market overview: a sector tile map + the news feed + cross-symbol
  **co-movement** (how each symbol moves with a reference over recent bins). A
  watch surface, not a trading grid.
- **LLM** — the assistant, receiving a UI-agnostic `AssistantContext` (a headline
  or a frozen book window) — the handoff contract, not an on-hot-path model.

The screen model uses a single **keymap table** that drives dispatch, the hint
line, *and* the generated `?` help — so help can't drift from the bindings — and
is rebindable via `RSX_TERM_KEYMAP` JSON.

## Assistant (agent runner)

The assistant behind the LLM screen is a **Claude Code agent** run per turn in
an isolated Docker container by arizuko's runner (`container.Run`, imported as
a Go library, unchanged) — rsx-term supplies only data: a prompt, an agent
folder, and a unix socket it hosts. `Run`'s stdout is discarded; the agent's
reply and resume id come back over that socket as a newline-framed `submit_turn`
message, so rsx-term's handler demuxes `submit_turn`/`submit_status` before
serving MCP on the same socket. Three channels feed the agent: a per-message
**vantage block** (the screen + focus + folded state the trader was looking at,
a pure provider generalizing `assistant/prompt.go`); an in-process **MCP
server** giving live read tools for every screen (off an atomic `StateMirror`)
and control verbs delivered through `p.Send` as **semantic messages** the same
state machine handles — never synthesized `tea.KeyMsg` — so the fat-finger hard
block and confirm gates bind the agent identically (order entry stops at a
ticket the trader key-confirms; the agent never executes); and a mounted
**agent folder** (`assistant/agentdir/`) whose CLAUDE.md/PERSONA.md carry the
standing RSX literacy. Multi-turn rides Claude Code's own session resume (the
`submit_turn` session id), tracked as a `thread→sessionID` map. Selected by
`RSX_TERM_ASSIST` (unset = the offline placeholder, unchanged); the container's
egress is currently open — the design's sharpest open question. Full design:
`specs/2/60-terminal-assistant.md`.

## Invariants & trust model

- **Offline by default.** The mock/default path makes zero network calls; every
  dial site (RSX WS, HL WS+meta, Tree of Alpha WS) sits behind an env opt-in
  (`RSX_TERM_VENUE`, `RSX_TERM_NEWS`) and a named goroutine.
- **The public feed is untrusted** (spec 4-cast §10.4): market-data frames are
  validated at the `conn` edge — malformed sides/prices are skipped, not coerced.
- **Never fabricate a number** — dash the unknown, label estimates (`~`), withhold
  on overflow, hard-block over the fat-finger cap. Full list in `notes/honesty.md`.
- **The classic DOM view is byte-locked** by a golden test (`TestDomViewGolden`),
  so the streaming work never regresses it.

## Architectural decisions (and the roads not taken)

- **Text, not a GPU bitmap.** Bookmap-class tools tie their edge to 40–600 fps GPU
  rendering; a character grid runs anywhere, over SSH, at trivial cost, and — because
  the render is a pure function of the model — the same compressed model can drive a
  bitmap frontend *later* without rewriting the core. The cost (coarser resolution)
  is bought back by the calibrated glyph bank + colour.
- **Keyboard-only, no mouse.** A mouse in a fast trading surface is a fat-finger and
  latency liability; the fisheye cursor + game-entry keys cover everything click did.
- **The DOM view stays, behind a flag.** The streaming heatmap is opt-in
  (`RSX_TERM_STREAM=1`); the original three-column DOM view remains the default and is
  golden-locked. Two renderers, one model.
- **A fixed grid repainted in place, not a scrolling log.** History lives in the
  model's ring, not terminal scrollback — so "the tape flies off the top" can't
  happen and the layout is stable.
- **Pure, clock-free `book/`.** State folds take timestamps as input and render
  nothing — the reason the heatmap is testable, the DOM view is golden-lockable, and
  freeze/replay is even possible.

## Edge cases

Order link down (amber dot, orders blocked, auto-reconnect) is independent of
marketdata down (degraded-book message, not a fake ladder); the two links flap
separately. A venue with a `nil` Submitter is read-only and says so. Startup with
a briefly-unavailable MD socket still reconnects rather than staying dark.

## How to read this crate

- **README.md** — what it is, why, how to run it.
- **ARCHITECTURE.md** (this) — how it's built.
- **notes/** — why each design choice, one file per decision.
- **SCREENS.md / VISUALS.md / FLOWS.md** — what each screen shows, the visual
  language, and the end-to-end user journeys.
