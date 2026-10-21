---
status: draft
---

# 16 — Open decisions

These are the decisions the founder still owes before or while glass absorbs
the terminal: each names what the terminal did, what the founder has said, and
the default glass takes until the founder answers. A decision the founder has
already made is not here; it is a requirement in the file that owns it, with
its quote in [15-founder-requirements.md](15-founder-requirements.md).

Source pin: citations are at `5d175b6ff04755e02fcca5ebbb918b01b929c23b`.

| ID | Question | Terminal | Founder so far | Glass default until decided |
|---|---|---|---|---|
| OD-01 | Does glass accept mouse input? | keyboard-only; mouse removed [code rsx-term commit 92bf9817] | "mouse is ppis9n" (terminal, F-KEY-05); parity "in the same way" (F-SURF-04); nothing on glass | keyboard-only; window controls (close, resize) only |
| OD-02 | How do far (aggregate) heatmap rows look different from live rows? | not dimmed in code; labelled in the gutter | "Far rows show aggregated liquidity labelled as such" (F-BOOK-01, rule of 2026-10-21) | a visible band separating far from live, plus the gutter labels |
| OD-03 | Is the order-entry view a separate screen, or are its verbs (typed price, TIF cycle, market, flatten, reverse, cancel-selected, cancel-all, ARMED, trace HUD) folded into BOOK? | a separate default view with a hard-coded keymap; `x` and `esc` mean different things on the two views | parity (F-SURF-04); the element inventory tags the view "cuttable, legacy"; no ruling | a fifth screen in the tab cycle, keys from the one table, conflicts resolved there (KM-22) |
| OD-04 | Which key opens the chat from every screen? | none | chat on every screen, one keypress (F-AST-03) | unbound until chosen; must not collide with any screen's table |
| OD-05 | How does glass get a trading token in production? | self-minted HS256 from a shared dev secret | identity is `rsx-auth` (spec 54); tokens reusable until `exp` | dev: self-minted against a local cluster only; production: an `rsx-auth` token, never minted by the client |
| OD-06 | Transport: QUIC + protobuf (founder's stated target) or the running JSON-over-WebSocket order channel? | JSON text over WS; protobuf md over WS | "just mark to support only protobug and quick for the tui !" (F-ACC-04); never withdrawn | speak what the gateway serves (JSON over WS) behind one connector, so a QUIC/protobuf connector is a swap |
| OD-07 | Merge the own-order marks (`◇` map, `▲`/`▼` ruler, `▸` ladder) into one treatment? | three treatments | design-system docs exist "to minimize the elements" (F-DS-04) | one outlined-cell treatment in Accent, side by shape on the ruler |
| OD-08 | Show correlated symbols' trades on BOOK? | not built | "shoudl show trades on correlated symbols" (F-SCR-04) | not built until the founder picks which symbols correlate (the co-move reference is a candidate) |
| OD-09 | What reference does a NEWS tile move against? | the oldest retained mid, ~6.4 s ago, called "session reference" in a comment | "prices are where they move relative to a reference" (F-SCR-05) | keep ~6 s and label it on screen (NW-01) |
| OD-10 | Switchable watchlists? | one list per venue; no key switches lists | lists belonged to the removed PAIR screen (F-SCR-06) | keep one list per venue, bind none, drop `list` from the mode line unless lists switch |
| OD-11 | May the agent propose an order (confirm-gated ticket)? | no control tools exist | "not a trading automaton" (F-AST-05); spec 60 question 1 | navigation and focus only |
| OD-12 | Agent container egress | n/a (HTTP client to an arizuko deployment) | spec 60 question 0: open internet by default; recommendation to restrict to Anthropic-only before a real account | runner disabled against real accounts until restricted |
| OD-13 | Wallet management scope | adjust collateral only | "make sur you have a walet management there in tui and glass" (F-SCR-08) | the USERS detail plus collateral history when the gateway serves it |
| OD-14 | ELI13 tooltip walkthrough: where, and from which docs? | none | requested 2026-07-20 (F-DEM-03) | after the screens ship; tooltips sourced from the repo docs once they describe glass |
| OD-15 | Label fixture / demo data on screen? | fixture accounts unlabelled; the demo scene is scripted | never stated | label any fixture source (`fixture` tag in the header) |
| OD-16 | Glass's environment variable names | `RSX_TERM_*`, `RSX_TUI_*`, `RSX_GW_URL`, `RSX_MD_URL` | config via env only (repo rule) | keep `RSX_GW_URL` / `RSX_MD_URL` / `RSX_TUI_THEME` / `RSX_SESSION_OPENS`; new names under `RSX_GLASS_*` |
| OD-17 | Prune list from the element inventory: fold the three violets (Heading / Accent / Ring), drop `◀ ▶`, drop the idle rail `│`, give news severity four distinct shapes, keep or fold the tape rail, merge the trace HUD and the speed strip | all present | design system kept "so … we could … minimize the elements and optimize" (F-DS-04) | keep all channels; apply only merges that lose no channel, each with review |
| OD-18 | ARMED banner contrast (2.35:1 default, 1.74:1 colourblind) | bright on red | none | page ink on the red ground (7.28:1 / 9.79:1) |
| OD-19 | Access paths: the terminal had SSH forced-command dispatch and a playground PTY embed; glass is a desktop GL window | SSH + web PTY | "users will just run it on thier computers... ideally also prpvide some containerized version" (F-ACC-01); web terminals removed (F-SURF-07) | a local desktop binary; a container image as the optional packaging; no SSH, no web |
| OD-20 | Multi-market screens (account, options, lending) from spec 55 | mocks only, all "[needs server]" | none since spec 55 | not built; the account view folds into USERS / positions when data exists |
| OD-21 | Present mode: the founder chose Mailbox ("yes do 2. with mailbox", F-GL-02); glass v1 is go-gl vsync-on and calls Mailbox a later go-webgpu swap | n/a | Mailbox, 60 Hz now, 240 Hz in v2 | vsync-on until the go-webgpu backend exists; the founder confirms or asks for Mailbox now |
| OD-22 | Accessibility surface (screen reader, focus, IME) | the terminal inherited the terminal emulator's | none | none in v1; colourblind theme and shape redundancy only |
| OD-23 | Glass layout: fixed 72 × 24 letterboxed, or derived from the window like the terminal | the terminal re-derived its grid from the window size | "left and vottom would be most important things.. so that small teeminals would work too" (F-SCR-01) | derive the grid from the window (every screen, not only BOOK) |

## Conflicts resolved by the newest statement (not open)

| Topic | Earlier | Newest (the rule) |
|---|---|---|
| web trading surface | 2026-07-23 07:03 "usable as a front to thr exchnage"; `TODO.md` 2026-07-23 promotes the playground to a second browser trading surface | 2026-07-23 07:21 "never dashboard"; 2026-10-21 web UIs removed, the playground is a demo and test tool (F-SURF-03, F-SURF-07) |
| default server | 2026-07-08 "hardcode rsx.krons.cx" (F-ACC-02) | 2026-10-21 "Production is explicit" (F-ACC-03) |
| BOOK windows | 2026-07-10 10 s / 60 s / 120 s / 300 s / 600 s / hours | 2026-07-23 3 s / 60 s / 1 h / 24 h (F-BOOK-03) |
| context per message | 2026-07-11 14:52 "based on where the user is looking" | same minute: "just make it for now ruch and full" (F-AST-03) |
| SSH access | 2026-07-05 SSH / wish dispatch | 2026-07-10 "no need for an ssh i gues" (F-ACC-01) |
| safety list | 2026-07-08 18:19 "cut all complience and alignment slop" | 18:22 "what youre doing is good. continue" (F-HON-01) |
| the UI | 2026-07-01 TUI not GUI | 2026-10-21 glass is the one UI (F-SURF-05) |
