---
status: draft
---

# 01 — Product

Glass is the RSX exchange's one trading UI: a native desktop window that shows
the whole order book and its recent history as a live liquidity heatmap,
lets a trader place, cancel and manage orders and positions from the keyboard
in single keystrokes, shows how fast every operation was, puts market context
and an LLM assistant one key away, and lets support staff manage users and
wallets — all honest about which numbers are exact, which are estimates and
which are unknown. It absorbs everything the `rsx-term` text terminal did
[founder F-SURF-04, F-SURF-05]. The rest of the spec set is listed in
[index.md](index.md).

Source pin: citations are at `5d175b6ff04755e02fcca5ebbb918b01b929c23b`.

## Who it is for

| User | Uses | Source |
|---|---|---|
| discretionary trader on RSX perps | BOOK: read order flow on the heatmap, quote by hand with size presets and a price cursor, take with one shifted key; position, uPnL and speed always visible | [doc rsx-term/README.md "When NOT to use this"], [founder F-KEY-01, F-KEY-02] |
| hand market maker | BOOK across 10-20 watched symbols, hopping by letter code | [founder F-KEY-11] |
| a trader who wants the classic ladder | the order-entry view: typed price, TIF, two-step confirm, flatten, reverse | [05-screen-order-entry.md](05-screen-order-entry.md) |
| a watcher of other venues | the same screens read-only over Hyperliquid or Phoenix | [founder F-VEN-01, F-VEN-02] |
| support and operations staff | USERS: directory, roles, accounts, freezes, collateral adjustments, notes — permissions enforced by the server | [founder F-SCR-08] |
| someone learning order flow | the heatmap teaches spoofing (a wall that appears and vanishes before price reaches it) and absorption (trades pound a level that does not shrink) | [doc rsx-term/DEMO.md "The two patterns to look for"] |
| a demo audience | a recorded walkthrough with every keystroke shown | [founder F-DEM-01, F-KEY-07] |

Design stance: speed and directness over learnability ("learning curve not a
consideration here as anyone can earn a few shortcuts", F-KEY-10), while every
keypress gives immediate, legible feedback ("a 'slot machine experience' but
for a trader that is in charge", F-KEY-09).

## What it is not

| Not | Why | Source |
|---|---|---|
| an automated or HFT client | at the exchange's microsecond latencies a human cannot compete; that is what the API is for | [doc rsx-term/README.md "When NOT to use this"] |
| a charting package | no candles, indicators or drawing tools: it shows order flow, not technical analysis | [doc rsx-term/README.md] |
| an everything-terminal | a few focused screens | [doc rsx-term/README.md] |
| an operator console | no process control, sessions, topology, log viewer, WAL inspector or fault injection; those are the playground's and `rsx-cli`'s | [doc .ship/33-TUI-SPEED-TESTS/PLAN.md "The TUI is a trading terminal, not a cluster console"], [founder F-SURF-03 "admin is the playground mostly so"] |
| a trading automaton behind a chat | the assistant chats and navigates; it never sends an order | [founder F-AST-05] |
| a latency benchmark | the speed readout is a demo of the path; the exchange's latency claims come from benches and gates | [doc CLAUDE.md "Latency & performance verification"] |

## The boundary with the playground

| Surface | Owns | Never | Source |
|---|---|---|---|
| glass | trading, positions, wallets, users and roles (as a client of the gateway and `rsx-auth`) | process control, WAL, cluster ops | [founder F-SURF-03 "the frontend to trade is glass and tui! never dashboard."], [doc rsx-term/CLAUDE.md "the dashboard is never a trading front"] |
| playground | the demo and test tool: start/stop processes, inspect WAL, inject faults, the demo scene, the dashboard walkthrough | a trading front; re-shipping exchange tools that belong in a CLI or library | [founder F-SURF-07 "playgtound is a demo and testing tool"], [founder F-SURF-08] |
| CLI / libraries | every exchange tool glass or the playground needs | | [founder F-SURF-08] |

Glass never depends on the playground being up. The superseded stances (a
browser trading surface in the playground, 2026-07-23; a web terminal at
`/terminal`) are both removed from the repo; they are listed with their dates
in [16-open-decisions.md](16-open-decisions.md) § conflicts.

## Screens

| Screen | For | File |
|---|---|---|
| BOOK | the heatmap and single-key trading on one symbol | [04-screen-book.md](04-screen-book.md) |
| order entry | the typed-price ladder, confirm, flatten, reverse, trace | [05-screen-order-entry.md](05-screen-order-entry.md) |
| NEWS | sector moves, co-movement, the headline feed | [06-screen-news.md](06-screen-news.md) |
| LLM | the assistant chat with a frozen-context handoff | [07-screen-assistant.md](07-screen-assistant.md) |
| USERS | users, roles, accounts, wallet actions | [08-screen-users.md](08-screen-users.md) |

One key apart (`tab` / `shift+tab`); shared chrome in
[09-chrome.md](09-chrome.md). Whether the order-entry view stays a separate
screen is OD-03.

## Guarantees (keeper invariants)

| ID | Guarantee | Source |
|---|---|---|
| P-01 | Keyboard-only. | [doc rsx-term/CLAUDE.md], [founder F-KEY-05]; glass: OD-01 |
| P-02 | Never fabricate a number: dash the unknown, mark estimates `~`, hard-block over the caps, withhold on overflow ([02-design-system.md](02-design-system.md) H-1 … H-12). | [doc rsx-term/notes/honesty.md], [founder F-HON-01] |
| P-03 | Offline by default: no network call without an explicit opt-in per dial site; the offline mock is the default for both channels. | [doc rsx-term/CLAUDE.md], [founder F-VEN-03] |
| P-04 | Production is explicit: no default dials a hosted deployment. | [founder F-ACC-03] |
| P-05 | The public feed is untrusted; validate at the connector. | [doc rsx-term/notes/honesty.md rule 6], spec 4 §10.4 |
| P-06 | The model is UI-agnostic and clock-free (`rsx-core`); the render is a pure function of the newest model state. | [doc rsx-term/CLAUDE.md], [doc rsx-glass/CLAUDE.md] |
| P-07 | Honest compression: far rows are aggregate windows labelled as such; no replay buffer; no lead-lag over jittery 100 ms mids. | [doc rsx-term/CLAUDE.md "Honest compression"] |
| P-08 | Show the speed: every operation's net, internal and engine time from the records' own stamps, measured or `—`. | [founder F-LAT-01, F-LAT-06] |
| P-09 | Generic multi-venue through one seam; RSX first-class. | [founder F-VEN-01, F-VEN-03] |
| P-10 | Minimal: the terminal's element set, no invented visuals; prune with review. | [founder F-GL-03 "keep it minimal like the terminal"] |
| P-11 | Developed against tests: every flow a test; goldens lock the views; full-stack tests measure speed. | [founder F-TST-01 … F-TST-04] |
| P-12 | The fixed-point rule: raw i64 on the wire, human decimals only at display and input. | [doc CLAUDE.md "Fixed-Point Arithmetic"] |

## Why a heatmap, why a keyboard

The open-source finance-UI genre is quote-plus-candles; a depth visualiser
for the book over time is the GUI incumbents' ground (Bookmap, Jigsaw, Sierra
Chart), heavy, closed and single-box, and by their own documentation
hidden-order and absorption signals are reactive and probabilistic. The gap
the terminal filled was honest presentation, fewer windows and keyboard
speed [doc rsx-term/README.md "Why it exists"]. Glass keeps that and adds what
a character cell could not: real rectangles, sub-cell resolution and colour
without a font lottery [doc rsx-term/notes/element-inventory.md "The GPU lens"],
at the terminal's simplicity [founder F-GL-01].
