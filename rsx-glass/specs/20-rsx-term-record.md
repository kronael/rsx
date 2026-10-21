---
status: reference
---

# 20 — The rsx-term record

This file is the inventory of what `rsx-term` was at the moment it was
captured: every tracked file and where its content lives in these specs, every
screen, flow, key binding and test file and where each is captured, what
could not be captured and why, the defects found while studying it, and the
references elsewhere in the repo that break when it is deleted.

## Recovering the code

Pinned commit: `5d175b6ff04755e02fcca5ebbb918b01b929c23b` (HEAD of the repo on
2026-10-21 when this record was written; `rsx-term/` was last changed in
`e6e190870614ccb405f71335cc2a77f133112481`). Every `rsx-term/<path>` citation
in these specs is at that commit:

```sh
git show 5d175b6ff04755e02fcca5ebbb918b01b929c23b:rsx-term/ui/stream.go
git archive 5d175b6ff04755e02fcca5ebbb918b01b929c23b rsx-term rsx-core | tar -x -C /tmp/rsx-term-at-pin
cd /tmp/rsx-term-at-pin/rsx-term && go test ./...        # needs rsx-core beside it
cd /tmp/rsx-term-at-pin/rsx-term && RSX_GW_URL=mock RSX_TERM_STREAM=1 go run .
```

The second command was used for this record: rendering the NEWS, LLM and
USERS frames quoted in files 06-08 from the archived tree. Go modules:
`github.com/charmbracelet/bubbletea v1.3.10`, `github.com/charmbracelet/lipgloss v1.1.0`,
`github.com/coder/websocket v1.8.15` (plus their indirect deps in `go.sum`).

Not in git (local files under `rsx-term/` that deletion removes for good):
`demo/term-live.cast` (the asciinema recording, ~3.4 MB) and
`demo/term-live.gif` (the unoptimised GIF) — `demo/term-live-opt.gif` IS
tracked; `tmp/screens/*.txt` (stale screen dumps from an earlier schedule;
regenerate with `RSX_TERM_DUMP=1 go test ./ui -run TestDumpScreens` at the pin);
`.claude/tmp/*` (empty session markers).

## File coverage

Every tracked file at the pin (line counts measured at the pin):

| File | Lines | Captured in |
|---|---|---|
| `README.md` | 133 | 01 (purpose, guarantees, non-goals), 13 (venue selection), 14 (env) |
| `ARCHITECTURE.md` | 158 | 01, 03, 13, 18; its assistant section describes spec 60 (07, 14) |
| `CLAUDE.md` | 146 | 01 (rules), 02 (encoding laws, keeper invariants), 15 (rules dated 2026-10-21) |
| `DEMO.md` | 92 | 01 (order-flow lesson), 11 (demo walk) |
| `FLOWS.md` | 135 | 11 (flows 1-15 one-to-one) |
| `ONEPAGER.md` | 61 | 01 |
| `SCREENS.md` | 259 | 04-09 (element inventory per screen, reconciled with the code) |
| `VISUALS.md` | 381 | 02, 03 |
| `notes/README.md` | 24 | this table |
| `notes/assistant.md` | 68 | 07, 14 |
| `notes/compression.md` | 94 | 03 |
| `notes/element-inventory.md` | 160 | 02 (tag column), 16 (OD-07, OD-17) |
| `notes/glyph-bank.md` | 111 | 02 (calibration), 18 |
| `notes/honesty.md` | 68 | 02 (H-1 … H-12) |
| `notes/keycast.md` | 50 | 09 |
| `notes/trade-side.md` | 48 | 02, 03 |
| `notes/venue-seam.md` | 79 | 13 |
| `main.go` | 757 | 13 (venues, symbols), 14 (configuration), 18 |
| `ui/model.go` | 492 | 04 (per-market state), 10 (screen cycle), 14 (config) |
| `ui/update.go` | 1299 | 04 (game entry, `fire` excerpt), 05 (`arm` and paths), 10 (dispatch), 12 (lifecycle, RTT), 13 (folds, md age) |
| `ui/stream.go` | 959 | 02 (glyph table, contrast functions), 03 (tiers, cell precedence), 04, 09 (mode line excerpt) |
| `ui/view.go` | 852 | 05 |
| `ui/news_view.go` | 817 | 06 (co-move excerpts), 07 |
| `ui/users_view.go` | 564 | 08 |
| `ui/keymap.go` | 367 | 10 (table excerpt, rebind rules) |
| `ui/keycast.go` | 189 | 09 (excerpt), 10 |
| `ui/market.go` | 174 | 06 (moves), 10 (code assignment excerpt) |
| `ui/format.go` | 153 | 02 (`fmtNs`, `fmtDec` excerpt), 05 (`parseRaw` excerpt) |
| `ui/styles.go` | 124 | 02 (palette, `UseTheme` excerpt) |
| `ui/sparkline.go` | 35 | 05 (excerpt) |
| `conn/live.go` | 493 | 12, 13 (`dialMd` excerpt) |
| `conn/hyperliquid.go` | 436 | 13 (contract table, sector excerpt) |
| `conn/phoenix.go` | 350 | 13 |
| `conn/mock.go` | 129 | 13 (script summary) |
| `conn/jwt.go` | 65 | 12 (claims excerpt) |
| `conn/symbols.go` | 85 | 13 |
| `news/news.go` | 51 | 13 |
| `news/treeofalpha.go` | 228 | 13 (severity excerpt) |
| `news/context.go` | 95 | 07 (struct excerpt) |
| `assistant/client.go` | 224 | 14 (SSE contract) |
| `assistant/prompt.go` | 173 | 07 (`Render` excerpt) |
| `support/support.go` | 165 | 08, 14 |
| `support/http.go` | 399 | 14 (contract excerpt) |
| `support/fixture.go` | 134 | 14 (fixture values) |
| `tools/glyphbank/glyphbank.py`, `README.md` | 153, 8 | 02 (measured coverage), 18; the tool itself is recoverable from git |
| `demo/walk.sh` | 163 | 11 (the walk), 18 |
| `demo/Makefile` | 37 | 18 |
| `demo/CLAUDE.md` | 66 | 11, 18 |
| `demo/term-live-opt.gif` | binary | recoverable from git; the face of the README |
| `go.mod`, `go.sum` | 32, 45 | this file (module list) |
| `ui/testdata/dom_view.golden` | 31 | 05 (quoted in full) |
| `ui/testdata/book_view.golden` | 29 | 04 (quoted in full) |
| tests (34 files) | 5,300 | 17 (every file mapped below), 11 (flow tables) |

## Screens

| Terminal screen / overlay | Captured in |
|---|---|
| BOOK (heatmap) | 03, 04 |
| classic order-entry view | 05 |
| NEWS | 06 |
| LLM | 07 |
| USERS | 08 |
| help overlay (streaming, generated) | 09, 10 |
| help overlay (classic, hand-written) | 05 |
| trace HUD (F3) | 05 |
| confirm preview, ARMED banner | 05 |
| keycast chip | 09 |
| symbol switcher, venue picker | 09, 10 |
| mode line, header, hint line, status line | 09 |
| PAIR (removed 2026-07-11, not at the pin) | 15 F-SCR-06; recoverable from git before `f5cc1b45` |

## Flows

`FLOWS.md` 1-15 → [11-flows.md](11-flows.md) rows 1-15 (same numbers and test
names). Streaming journeys S1-S25, connection flows C1-C10 and the demo walk
are in the same file; none of these had a `FLOWS.md` entry in the terminal.

## Key bindings

| Binding set | Captured in |
|---|---|
| streaming globals (`q`/`ctrl+c`, `tab`, `shift+tab`, `f9`, `r`, `p`, `?`) | 10, exact table excerpt |
| BOOK (`x n b s h/left l/right j k up down enter f d esc`), classes `1-5`, `⇧1-5` | 10 |
| NEWS (`/ j/down k/up enter esc`), class `a-z` | 10 |
| LLM (`esc`), chat typing class | 10 |
| USERS (`/ j/down k/up enter g o a z Z n esc`), prompt typing class | 10 |
| capture modes (switcher, picker, searches, chat, prompts) | 10 |
| classic view (`q ctrl+c ? esc enter 0-9 . backspace tab b s t r p + = - j k m up down c X x R f2 f3`) | 10, 05 |
| keycast labelling | 09, 10 |
| rebind JSON and action ids | 10 |

## Test files

| Test file | Tests | Captured in (17-acceptance) |
|---|---|---|
| `ui/flows_test.go` | 15 | AC-ORDER-02 |
| `ui/stream_test.go` | 20 | AC-HM-01 … 08, AC-BOOK-02 … 04, AC-KEY-04, AC-KEY-06 |
| `ui/stream_golden_test.go` | 2 | AC-BOOK-01, AC-NEWS-01 |
| `ui/dom_golden_test.go` | 1 | AC-ORDER-01 |
| `ui/update_test.go` | 24 | AC-ORDER-03, 05, 06, AC-CONN-08 |
| `ui/orders_test.go` | 27 | AC-ORDER-04, 08 … 12, AC-DS-02, 05, 06 |
| `ui/view_test.go` | 7 | AC-ORDER-11 … 13 |
| `ui/ladder_test.go` | 3 | AC-ORDER-07 |
| `ui/sparkline_test.go` | 4 | AC-ORDER-12 |
| `ui/keymap_test.go` | 7 | AC-KEY-01, 02, AC-NEWS-03 |
| `ui/keycast_test.go` | 6 | AC-KEY-05 |
| `ui/microscope_test.go` | 6 | AC-BOOK-05 |
| `ui/news_view_test.go` | 14 | AC-NEWS-01, 03, AC-LLM-01 … 03 |
| `ui/comove_test.go` | 4 | AC-NEWS-02 |
| `ui/users_view_test.go` | 8 | AC-USERS-01 |
| `ui/venue_test.go` | 4 | AC-CONN-02 |
| `ui/watchlist_test.go` | 6 | AC-BOOK-03, AC-KEY-03, 04 |
| `ui/palette_test.go` | 1 | AC-DS-01 |
| `ui/glyphs_test.go` | 2 | AC-DS-03 |
| `ui/screendump_test.go` | 1 | 18 (review gallery), AC-01 |
| `conn/live_test.go` | 7 | AC-CONN-01 |
| `conn/hyperliquid_test.go` | 8 | AC-CONN-03 |
| `conn/phoenix_test.go` | 6 | AC-CONN-03 |
| `conn/symbols_test.go` | 5 | AC-CONN-04 |
| `conn/jwt_test.go` | 3 | AC-CONN-05 |
| `conn/mock_test.go` | 3 | AC-CONN-06 |
| `news/treeofalpha_test.go` | 7 | AC-CONN-07, AC-03 |
| `news/context_test.go` | 2 | AC-LLM-05 |
| `assistant/client_test.go` | 4 | AC-LLM-03 |
| `assistant/prompt_test.go` | 3 | AC-LLM-04 |
| `assistant/smoke_test.go` | 1 | AC-CONN-11 |
| `support/http_test.go` | 5 | AC-USERS-02, AC-03 |
| `support/fixture_test.go` | 4 | AC-USERS-03 |
| `hl_smoke_test.go` | 1 | AC-CONN-11 |

Totals: 34 test files, 221 test functions (counted at the pin with
`grep -c '^func Test'`).

## What could not be captured, and why

| Item | Why | Where it survives |
|---|---|---|
| the Bubble Tea / lipgloss rendering code line by line | it is terminal-specific; the spec keeps behaviour, layout and encoding, and excerpts only generic or reference code (founder F-SURF-06) | git at the pin |
| exact ANSI escape output | colour is specified as tokens and contrast; goldens strip ANSI by design | git at the pin |
| `demo/term-live.cast`, `demo/term-live.gif` | untracked local files | nowhere after deletion (the optimised GIF is in git) |
| `tmp/screens/*.txt` | untracked, stale | regenerate from the pin |
| per-font glyph coverage for fonts other than DejaVuSansMono | never measured | the tool in git |
| the browser-embedded terminal and SSH access | they lived outside `rsx-term` and are removed from the repo; glass has no equivalent by decision | 18 (mapping), git history |
| founder rulings on mouse in glass, demo labels, vim keys, fonts | never stated | 16 (open decisions) |
| behaviour of the live demo cluster during recording | environment, not code | BUGS entries cited in 11 |

## Defects found in the terminal while writing this record

Not in `BUGS.md` at the pin (the queue is being edited concurrently; these are
for its owner to record). Each is a requirement in the file named.

| Defect | Evidence | Requirement |
|---|---|---|
| An unpaired fill is folded into the position as a BUY | `rsx-core/wire/order.go:358-365` sets `Buy`; `rsx-term/ui/update.go:106-114` applies it; `wire/order_test.go::TestFolderFillUnknownSidesFallBackToBuy` locks it | GW-21 (12) |
| A repeated status-1 `U` appends a duplicate working order | `rsx-term/ui/update.go:89-99` appends unconditionally | GW-25 (12) |
| Working orders keep the submitted qty after partial fills | `rsx-term/ui/model.go:198-208` | GW-26 (12) |
| Liquidation `Q` frames are dropped | `rsx-core/wire/order.go:247-258` | GW-27 (12) |
| Seq gaps are detected and ignored | `rsx-term/ui/update.go:50-66` discards `Observe`'s result | MD-10 (13) |
| BOOK header "mid" is the sticky axis anchor, not the live mid | `rsx-term/ui/stream.go:844-851` | BK-02 (04) |
| BOOK has no status line; order results are written but not drawn | `rsx-term/ui/stream.go:861-877` | BK-10 (04) |
| BOOK latency line shows RTT only, no legs | `rsx-term/ui/stream.go:949-959` | BK-01 (04) |
| The badge reads `RSX` on every venue; the link dot shows the RSX order link on read-only venues | `rsx-term/ui/stream.go:834-843` | BK-03, BK-04 (04) |
| Far rows are not dimmed although the docs say they are | `rsx-term/ui/stream.go:530-539` | HM-16 (03) |
| Symbol codes `j`/`k` are unreachable by letter on NEWS | `rsx-term/ui/market.go:74`, `rsx-term/ui/keymap.go:129-135` | KM-11 (10) |
| NEWS tiles move vs ~6.4 s ago while the code calls it the session reference | `rsx-term/ui/market.go:55-56, 152-160` | NW-01 (06) |
| The active watchlist can never change (`listSel` is never written) | `rsx-term/ui/model.go:294-297` | NW-03 (06) |
| USERS fetches fills and funding and never shows them | `rsx-term/ui/users_view.go:115-133, 316-337` | US-01 (08) |
| USERS add-note sends the note as the reason | `rsx-term/ui/users_view.go:172-177` | US-06 (08) |
| An offline accounts client shows an `error:` row and the stub note together | rendered at the pin | US-02 (08) |
| The streaming help overlay is one ~400-column line wide | rendered at the pin | US-08 (08) |
| `lot_size` is fetched and never used to align quantities | `rsx-term/conn/symbols.go:19-26` | MD-23 (13) |
| A zero-value `Accepted.RttNs` reads as a measured 0 ns RTT (visible in the BOOK golden) | `rsx-term/ui/testdata/book_view.golden` | BK-17 (04) |
| The recomputed contrast matrix does not reproduce the documented 3.97:1 / 3.91:1 worst pairs | [02-design-system.md](02-design-system.md) "Measured matrix" | DS-10 (02) |
| `specs/2/49-webproto.md` calls the running JSON frames removed | spec 49 vs `rsx-gateway/src/records.rs` | 12 (header note) |
| The repo `Makefile` comment says the terminal defaults to the hosted deployment; the code defaults to a local gateway | `Makefile:280-284` vs `rsx-term/main.go:34-38` | SR-22 (18) |

## References that break when `rsx-term/` is deleted

`git grep -l rsx-term` at the pin outside the directory: 80 files. Grouped:

| Group | Files |
|---|---|
| Go code importing or naming it | `rsx-glass/*.go` comments (`grid.go`, `heat.go`, `feed.go`, `demo.go`, `accounts.go`, `atlas/atlas.go`, `soft/soft.go`, `live/live.go`, `mockx/wire.go`, tests), `rsx-core/book/basis.go` comment, `rsx-maker/maker.go` comment |
| glass docs | `rsx-glass/README.md`, `SPEC.md` (re-pointed by this change), `CLAUDE.md`, `Makefile` |
| repo docs | `CLAUDE.md`, `README.md`, `ARCHITECTURE.md`, `PROGRESS.md`, `BLOG.md`, `BUGS.md`, `docs/demo.md`, `demo/CLAUDE.md`, `demo/Makefile`, `demo/run.sh`, `demo/exchange-walkthrough.sh`, `reports/20260723_gateway-egress.md` |
| specs | `specs/2/1-architecture.md`, `49-webproto.md`, `55-terminal.md`, `58-http3.md`, `59-latency-observability.md`, `60-terminal-assistant.md`, `index.md`, `rsx-core/SPEC.md` |
| playground | `rsx-playground/CLAUDE.md`, `README.md`, `demo_scenario.py`, `docs/README.md`, `docs/tabs.md`, `pages.py`, `server.py`, `terminal.py`, `terminal_page.py`, `tests/api_e2e_test.py`, `tests/play_terminal.spec.ts`, `tests/playwright.config.ts` |
| other crate docs | `rsx-marketdata/CLAUDE.md` |
| build | `Makefile` targets `term`, `term-local`, `term-demo`, `term-smoke-llm` |
| history (leave as is) | `.diary/*`, `.ship/42-TERM-AUDITS/*`, `.ship/44-TERM-SUPERIOR/PLAN.md`, `.ship/45-ARIZUKO-LLM/*`, `.ship/48-*/PLAN.md`, `.ship/53-COMPONENT-COMPLETION/*`, `.ship/plan-book-matching-publication.md` |

Some of these are already being changed by the concurrent removal of the web
UIs; the list is the state at the pin.
