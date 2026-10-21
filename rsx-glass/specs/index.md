# rsx-glass specs

These specs are the complete product, design-system, screen, flow, keymap,
contract and acceptance specification for glass as the RSX exchange's one
trading UI, written by studying the `rsx-term` text terminal (code, tests,
docs) and the founder's recorded requests before `rsx-term` is deleted. Each
file answers one question:

| File | Status | The question it answers |
|---|---|---|
| [01-product.md](01-product.md) | draft | What is the UI for, who uses it, what is it not, where does it stop and the playground begin? |
| [02-design-system.md](02-design-system.md) | draft | Which colours, contrast rules, shapes, type and honesty markers does every screen draw with? |
| [03-heatmap.md](03-heatmap.md) | draft | How does the book and its history become cells: time, price and size axes, cell precedence, gutter, rails, microscope? |
| [04-screen-book.md](04-screen-book.md) | draft | What is on BOOK, from which data, in which states, and what does each order key send? |
| [05-screen-order-entry.md](05-screen-order-entry.md) | draft | What is the classic order-entry view: ladder, form, confirm, position, risk row, speed strip, trace HUD? |
| [06-screen-news.md](06-screen-news.md) | draft | What is on NEWS: sector map, co-movement, the feed, search, handoff? |
| [07-screen-assistant.md](07-screen-assistant.md) | draft | What is on the assistant screen, what does a handoff carry, how do threads and turns behave? |
| [08-screen-users.md](08-screen-users.md) | draft | What is on USERS: directory, detail, action prompts, permissions? |
| [09-chrome.md](09-chrome.md) | draft | What do all screens share: header, mode line, status and hint lines, help, keycast, switcher, picker? |
| [10-keymap.md](10-keymap.md) | draft | Which key does what on every screen and in every capture mode, and how are keys rebound? |
| [11-flows.md](11-flows.md) | draft | Which whole user journeys exist, and which test drives each? |
| [12-contract-gateway.md](12-contract-gateway.md) | draft | What frames go to and come from the gateway, how are they folded, which reason codes exist, how is latency derived? |
| [13-contract-marketdata.md](13-contract-marketdata.md) | draft | How do market data, the other venues, symbol config and news reach the UI? |
| [14-contract-support-assistant.md](14-contract-support-assistant.md) | draft | Which REST, identity, support and assistant contracts does the UI use, and what is its configuration? |
| [15-founder-requirements.md](15-founder-requirements.md) | reference | What did the founder ask for and correct, verbatim, when and where? |
| [16-open-decisions.md](16-open-decisions.md) | draft | Which decisions does the founder still owe, and what does glass do until then? |
| [17-acceptance.md](17-acceptance.md) | draft | Which checks prove a glass implementation absorbed the terminal? |
| [18-terminal-mapping.md](18-terminal-mapping.md) | reference | What existed only because rsx-term ran in a terminal, and what replaces it? |
| [19-glass-status.md](19-glass-status.md) | reference | What does glass already have, and what does it lack? |
| [20-rsx-term-record.md](20-rsx-term-record.md) | reference | What was rsx-term file by file, where is each part captured, what could not be, what breaks on deletion? |

Read 01 → 02 → 03 → the screen files → 10 → 11 → the contracts; 15 and 16
are the decision record; 17 is the test plan.

## Recovering the deleted rsx-term code

Every `rsx-term/<path>` (and `rsx-core/<path>`) cited in these files is at the
pinned commit `5d175b6ff04755e02fcca5ebbb918b01b929c23b`. After `rsx-term/` is
deleted, recover any file with:

```sh
git show 5d175b6ff04755e02fcca5ebbb918b01b929c23b:rsx-term/<path>
```

or the whole tree, runnable, with
`git archive 5d175b6ff04755e02fcca5ebbb918b01b929c23b rsx-term rsx-core | tar -x -C <dir>`.
Line numbers in citations are at that commit. Files that were never in git
and so cannot be recovered are listed in
[20-rsx-term-record.md](20-rsx-term-record.md).

## Conventions in these files

- **Source tags.** `[code <path>:<lines>]` code at the pin; `[doc <path>]` a
  document at the pin; `[founder F-…]` an entry in
  [15-founder-requirements.md](15-founder-requirements.md) with its quote, date
  and session; `[BUGS <id>]` an entry in the repo `BUGS.md`. Every requirement
  row carries at least one.
- **Excerpts.** Generic and reference code — heatmap binning, fisheye
  mapping, glyph and ramp tables, the keymap table, frame decoding, the fold
  logic, flow test cases — is quoted exactly, with its path and lines, because
  the founder asked for excerpts where the code is generic or a reference
  (F-SURF-06). Everything else is cited by path at the pin.
- **Terminal behaviour vs founder wish.** Where they differ, both are recorded
  and the founder's wish is the requirement; the terminal's behaviour is
  stated as "at the pin".
- **Requirement IDs.** Per file prefix (`DS-`, `HM-`, `BK-`, `OE-`, `NW-`,
  `AS-`, `US-`, `CH-`, `KM-`, `GW-`, `LAT-`, `MD-`, `SR-`, `P-`, `H-`); open
  decisions `OD-`; acceptance `AC-`.

## Related specs elsewhere in the repo

`specs/2/55-terminal.md` (the terminal UX contract, partly stale: it still
lists a PAIR screen and an older time schedule), `specs/2/60-terminal-assistant.md`
(the assistant's runner and MCP design, draft, owner of that design),
`specs/2/49-webproto.md`, `specs/2/59-latency-observability.md`,
`specs/2/7-dashboard.md`, `specs/2/26-rest.md`. Where one of them disagrees
with the code at the pin, these files follow the code and say so.
