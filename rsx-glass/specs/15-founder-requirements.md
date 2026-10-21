---
status: reference
---

# 15 — Founder requirements ledger

This ledger is every request, correction and decision the founder stated
about the trading terminal and the glass UI, quoted verbatim (typos kept),
with its date, its session, the requirement it implies, and the spec file that
carries it. Where a later statement overrides an earlier one, both appear and
the later one is marked as the rule. The other spec files cite these entries
by ID (`F-…`).

How this was gathered: user messages extracted by script from the session
transcripts under `~/.claude/projects/-home-onvos-app-rsx/` and the earlier
checkout `~/.claude/projects/-home-onvos-sandbox-rsx/`, `~/.claude/history.jsonl`,
the Codex rsx sessions, the repo `.diary/`, the project diary, memory, `.ship/`
plans, `BUGS.md` and commit bodies (`recall-memories` procedure). Evidence line:
sessions 18 Claude + 4 interactive Codex read · founder messages 1109 Claude +
208 Codex · topic hits 429 · history.jsonl hits 599. Quotes were re-checked
against the raw extraction on 2026-10-21.

Session handles (UTC dates; full ids):

| Handle | Full session id | Where |
|---|---|---|
| `f8440a66` | `f8440a66-876f-4af8-8a1f-b69a089cbf93` | rsx, Jul 10-11 |
| `b95a6305` | `b95a6305-ee7e-4b03-8654-48a3c7bc11fc` | rsx, Jul 11-19 |
| `bca65731` | `bca65731-3eb5-44f6-bcbc-c0af9831fa53` | rsx, Jul 19-23 |
| `9fb69ebf` | `9fb69ebf-a7eb-4aa0-aee2-879be954b3a9` | rsx, Oct 20-21 |
| `1b29dbae` | `1b29dbae-cf40-49ee-abf3-e541c48736f4` | rsx, Oct 20-21 (twin of the above) |
| `sb:1b86cc72` | `1b86cc72-6ac9-4bb0-8fa2-ffb64af1b6e0` | sandbox checkout |
| `sb:27e9e5d0` | `27e9e5d0-524c-48e8-83c1-415fa82b4110` | sandbox checkout |
| `sb:53137f6a` | `53137f6a-cce4-4d60-9ded-3cad477c7dc7` | sandbox checkout |
| `sb:11825966` | `11825966-d82b-4722-9569-f1b917189917` | sandbox checkout |
| `sb:6fa85434` | `6fa85434-e476-4bd5-b6eb-a84a2bb19132` | sandbox checkout |
| `cx:019f4235` | `019f4235-f2d7-7b33-b264-94b6721348de` | Codex, Jul 8-9 |
| `cx:019f45dc` | `019f45dc-a665-7580-8ecf-9b56be94c48d` | Codex, Jul 9 |
| `cx:019fa4d9` | `019fa4d9-9435-7de3-af2b-aa8466e2d5ca` | Codex, Jul 27-Aug 5 |

## Surface decisions (newest last; the last row of each group rules)

| ID | Quote | When · where | Requirement | Captured in |
|---|---|---|---|---|
| F-SURF-01 | «I want a trading TUI.. not a gui !  and I want the tui to be done in the best tool available.... (mark GUI as to be discontinued now .. but the playgorund should have some less interactive gui on the web)» | 2026-07-01 08:39 · sb:1b86cc72 | trade surface was a TUI (overridden by F-SURF-07 for the trade surface) | 01 |
| F-SURF-02 | «i see thourhg this is the tui.. we only wahmnt one terminal.» · «term not tui» · «xompare tui and term. only keep.one.» | 2026-07-10 18:08-18:31 · f8440a66 | exactly one trading UI | 01 |
| F-SURF-03 | «the frontend to trade is glass and tui! never dashboard.» · «admin is the playground mostly so» · «it shoud be in tui and glass somehow...  not on the web.» | 2026-07-23 07:21-07:22 · bca65731 | trading and user management live in the native UI, never the web dashboard; overrides the same day's 07:03 «it should be usable as a front to thr exchnage and user and authorizaton manageemnt» | 01, 08 |
| F-SURF-04 | «ok does it have all featuews as the terminal one all of them is it completel in the dame way?» | 2026-10-21 07:27 · 1b29dbae | glass reaches full feature parity with rsx-term, in the same way | 01, 19 |
| F-SURF-05 | «do this now thrifuh an opus... distill and gsther from the code and from all the files and recall memories a xomplete spec and design system and all of the terminal ui.   and save it for the glass graphical ui.  then deletr the terminal one completely and fulky.  dont imolement anything just study spec and save. these will be the specs design system and sreens for hlthe complete glasss ui» | 2026-10-21 07:29 · 1b29dbae | this spec set; rsx-term is deleted after it | index, 20 |
| F-SURF-06 | «alsp take code excrprts whrre these are generic or revference files once delete dthey will be in git anyway» | 2026-10-21 07:29 · 1b29dbae | exact code excerpts for generic and reference code; the rest by path at the pinned commit | index |
| F-SURF-07 | «remove.completely all other uis yiu find in the source and references thereof... e.g. the web etc. keep the terrminal until trnadfered and the glass» then «no not the palyground playgtound is a demo and testing tool» | 2026-10-21 07:30-07:31 · 1b29dbae | glass is the one trading UI; web terminals go; the playground stays as a demo and test tool | 01, 18 |
| F-SURF-08 | «playground shiuld not reship stuff theat is elsewhwre snd all tolls useful for thr exchange shiuld be ij cli or as a lib accessible» | 2026-10-21 07:31 · history.jsonl | glass depends on CLI / library surfaces, never on the playground | 01, 14 |

## Interaction and keys

| ID | Quote | When · where | Requirement | Captured in |
|---|---|---|---|---|
| F-KEY-01 | «but hey... its ok its like a game.... what we need is ... one five keys various sized of passive orders... shift the same keys actove orders... one click is place the other clic is cancel.» | 2026-07-10 18:04 · f8440a66 | five size presets; shift = aggressive; single-action place and cancel | 04, 10 |
| F-KEY-02 | «on the book depth its for quiting.....» · «everytime trade is a single keypress actiom.» | 2026-07-10 18:43-18:44 · f8440a66 | BOOK is for quoting; one keypress per trade | 04 |
| F-KEY-03 | «reduce only are modifiers.. ehich are like toggled.  generally. displayed maybe at the top.of the screen or whrr...» | 2026-07-10 20:28 · f8440a66 | RO/PO are persistent toggles shown on the mode line | 09 |
| F-KEY-04 | «make.it ise all.keys to do good things.. also.build. ability to bind keys have prefixes. etc.  but onlynpractical .keep it minimal... also. waht is cursor selectoion?  all has to have help...» | 2026-07-10 20:51 · f8440a66 | rebindable keys, prefix chords, help for everything, practical and minimal | 10 |
| F-KEY-05 | «what click?  there should be no mouse» then «mouse is ppis9n» | 2026-07-11 06:30-06:31 · f8440a66 | keyboard-only (corrects F-KEY-01's "click") | 10 |
| F-KEY-06 | «button... jsut a keypress... on  every screen you can see what keys you can press on thr roght top or so...» | 2026-07-11 15:00 · b95a6305 | a per-screen key legend top-right | 10 (KM-21), 09 |
| F-KEY-07 | «on the terminal demo.. you have tlo show what keya are you hitting as an overlay dialog or so.. maybe the term has to have tjis as a geature... to enable» | 2026-07-12 18:30 · b95a6305 | keycast: a built-in, toggleable overlay | 09 |
| F-KEY-08 | «make sure the web terminal stels all keys well.» · «also arroews, etc.» | 2026-07-08 20:17-20:21 · cx:019f4235 | every key reaches the app: arrows, Tab, F-keys, Esc | 10, 18 |
| F-KEY-09 | «13yo ... ok... BUT.  make sure the terminal works and then a 13yo will just press buttons and see what happens. as long as its clear what did happen ... it all should be mostlyu like a "slot machine experience" but for a trader that is in charge, empowered, to have all relevant info and do all relevant actions» | 2026-07-09 08:02 · cx:019f45dc | every keypress gives immediate legible feedback in the status line | 09, 17 |
| F-KEY-10 | «simple fast usable... learning curve not a consideration here as anyone can earn a few shortcuts» | 2026-07-08 17:06 · sb:6fa85434 | optimise for speed over learnability | 01 |
| F-KEY-11 | «book vire is to do market making by hand.. place dome deep orders ... easily switch between individual symbols there... by combinations of letters somehow.maybe x introcudes a switch and then yiu type a bookmark or a some letter code of symbol and then it switches... aim is to be able to rapidly swotxh 10-20 symbols and watch them.» · «and trade them» | 2026-07-10 20:23 · f8440a66 | `x` + letter code symbol switch over 10-20 watched symbols | 04, 10 |

## Screens

| ID | Quote | When · where | Requirement | Captured in |
|---|---|---|---|---|
| F-SCR-01 | «possibly maybe even a line terminal trading.. whwre we would see the teading experience like a matrix one.. e.g. the timeline will be kept on the screen as a past compression ... and left and vottom would be most important things.. so that small teeminals would work too....» · «think of it as like eaxh line or several lines hold the state and then they drift up to keep the histoey....» | 2026-07-10 18:34-18:35 · f8440a66 | the BOOK heatmap; most important content bottom-left so small windows work | 04 |
| F-SCR-02 | «we need trades to eb visible and book.chsnges ideally.. e.g. size and intenzity ind epth... by color and sqrt or log compressions going back the book on toh sides...» | 2026-07-10 18:42 · f8440a66 | log size → colour; trades and book changes visible | 03 |
| F-SCR-03 | «you should just rrwdraw what needs to.. it can never be just logging terminal or so.» · «not all.of it shiuld be the book however. youd need to see what how bug and where trades took place.» | 2026-07-10 20:06-20:07 · f8440a66 | repaint in place; trades a co-equal layer | 02 (DS-30), 03 (HM-11) |
| F-SCR-04 | «alsp on the book.. shoudl show trades on correlated symbols... and ... news events.. with click out to expand into a news feed with a search and a click to go to an llm assistant with the context.» | 2026-07-10 18:46 · f8440a66 | news rail on BOOK; expand to a searchable feed; hand off with context | 04, 06, 07 |
| F-SCR-05 | «news... has a news feed we have the trre feed... and then only displays symbols and prixe .. its much more overvirwish than pairs... here symbols are in grids and prices are where they move relative to a reference..  the grid is orgsnixed by swctors» | 2026-07-10 20:34 · f8440a66 | NEWS: sector grid of moves vs a reference + the feed | 06 |
| F-SCR-06 | REMOVE the PAIR screen; «NEWS absorbs cross-symbol as **correlations + inter-symbol movement** (overview, not a trading grid)» (diary record of the founder's direction) | 2026-07-11 · `.diary/20260711.md` L79-87, commit `f5cc1b45` | no PAIR screen; cross-symbol is NEWS co-movement; no lead-lag | 06 |
| F-SCR-07 | «ok... im not really satisfied with the book view.. it should have... fast that it has then slower and then long term reference..» · «donr see why the whole books are green af.  possibly we need more nuance» | 2026-07-12 19:24-19:25 · b95a6305 | fast / slower / long-term tiers; more colour nuance | 03 |
| F-SCR-08 | «yes build the user and in tui and glass.. also make sur you have a walet management there in tui and glass» · «the accounts live in the db... but the api for this shiuld be the gateway right?» | 2026-07-23 07:41-07:46 · bca65731 | USERS screen with wallet management, data over the gateway API | 08, 14 |
| F-SCR-09 | «possibly i got too excited... and want to have two things now... one is the playground amd the other is the terminal. whixh will.work for the exchsnge in the first main screen and the other screens could perhaps be used if connected hyperliquid» | 2026-07-10 20:36 · f8440a66 | the main screen serves RSX; others work standalone on an external venue | 13 |

## BOOK windows and encoding

| ID | Quote | When · where | Requirement | Captured in |
|---|---|---|---|---|
| F-BOOK-01 | «each line updates arlt a.different pace... first few are online as fadt as youncan.. the next is 10s 60s 120s 300s 600s and hours... the further horizons should rather show liquidity than exact books» | 2026-07-10 20:03 · f8440a66 | log-time rows; far rows show liquidity (schedule numbers superseded by F-BOOK-03) | 03 |
| F-BOOK-02 | «long standing orders should be tracked and specially displayed. these now. however. dont seem to be completely satisfied by havijg them clculated on the ui... and they would need some support from the ebgine» | 2026-07-10 20:05 · f8440a66 | persistence mark now, engine order-age feed later | 03 (HM-10) |
| F-BOOK-03 | «for the term also... the books are flowing too fast.. i need 3x shoeter the shortest window on screen and then next window should be 20x as kuch time and the third 100x more... e.g. tje second is minites and the other is an hour and the agg is days reference open.   and the opens should be more.opens of the seesions... configurblable 4 open sessions... default is 0 utc us europe aisa.open hours.» then, on the proposal 100 ms / 3 s / 60 s / ~1 h / 1 day with 4 configurable opens, «ofc like go» · «also uodate tg glass in this direction too!» | 2026-07-23 06:07 and 07:02 · bca65731 | THE RULE: 3 s / 60 s / 1 h / 24 h far windows, four configurable session opens | 03 (HM-04, HM-05) |
| F-BOOK-04 | «shades and xolors...intensity and count.» · «bids and asks shiuld not be separated by color...» · «also you can use chars to modify the tiles... not just color and shade.» | 2026-07-10 18:48-20:09 · f8440a66 | colour = size, shape = count; side by position, never colour | 02, 03 |
| F-BOOK-05 | «create a glyph bank by making images of glyphs making analysis of the pictures» · «its not useful to see the exact sizes.. a quadratic or log.scale.thst is wuantized woth good basis and scaled stably is veryw ell eniugh.» | 2026-07-10 20:11-20:14 · f8440a66 | measured glyph ordering; quantised log scale on a stable basis; trade feed beside the map | 02, 03 |

## Speed and timings

| ID | Quote | When · where | Requirement | Captured in |
|---|---|---|---|---|
| F-LAT-01 | «i want the tui to also displat timings.. so as to shill the speed of the exchange.» · «that means evwry operation for the user should measure network and internal network and engines processing times» | 2026-07-03 12:13 · sb:1b86cc72 | net / internal / engine per operation | 12 (LAT-02), 05 |
| F-LAT-02 | «make it optionaly display detailed tracing... like an overdraw over a video game  ... using some fxxx shortcut.» | 2026-07-05 08:18 · sb:27e9e5d0 | F-key trace overlay | 05 (trace HUD) |
| F-LAT-03 | «I say yes. include processing times... but include them in normal market messages.. .alwayas. ... or. alsoinclude some telemetry long-term analytics message... but this would mostly be like allowing users to read prometheus mtrics off the systems.» | 2026-07-08 19:49 · sb:6fa85434 | per-hop stamps in the normal records; aggregates via Prometheus | 12 (LAT-03) |
| F-LAT-04 | «measure all latencies to marketdata too. and also the mark engine should come with latencies attached, estimates on how far before were the nubers observed....» | 2026-07-08 19:12 · sb:6fa85434 | market-data age and mark age shown | 13, 04 |
| F-LAT-05 | «lal the latency tables should habe p50 and p99 !» | 2026-07-27 19:07 · cx:019fa4d9 | every latency readout carries p50 and p99 | 09, 05 |
| F-LAT-06 | Standing rule carried into `rsx-term/CLAUDE.md`: "Show the speed: every operation reports net, internal and engine time from the records' own stamps; a latency is measured or it is `—`." | 2026-10-21 · commit `bfbd6530` | as F-LAT-01 with the honesty clause | 12 |

## Assistant

| ID | Quote | When · where | Requirement | Captured in |
|---|---|---|---|---|
| F-AST-01 | «the llm and news... just load something generic... treeofalpha feed for the nwwes and placeholder for the llm so far...» | 2026-07-10 18:47 · f8440a66 | Tree of Alpha news; honest placeholder when no model | 06, 07 |
| F-AST-02 | «the llm... shiuld mostly be a chatting interface...» | 2026-07-11 12:00 · b95a6305 | chat, not an automaton | 07 |
| F-AST-03 | «every page should have a chat input view. and then he shiukd ... like press a button switch to chat type a message.» · «but the fact he went there from say the depth view would send a digferent context then if he was in the news views...» · «but on the other count... just make it for now ruch and full» · «everything sent in the mcp» · «the mfoel just knows all screenn and perhaps more than thenuser sees» · «mcp is needed to be able to control everything and fetch everything» | 2026-07-11 14:52-14:53 · b95a6305 | chat key on every screen; a vantage block with every message, rich and full; MCP read + control of every screen | 07, 14, 10 (KM-20) |
| F-AST-04 | «maybe lets start by running it in thelocal dolder... not realky connectr to arizuko... inly reusing its claude code runner.» · «no fat messaging grnats erc.» · «yes this means ideally just run runner unchanged..» | 2026-07-11 14:54-14:59 · b95a6305 | the arizuko Claude Code runner, unchanged, local | 14 |
| F-AST-05 | Standing rule (commit `bfbd6530`): "The assistant is a chat on every screen: the message carries the vantage (what the user is looking at), the MCP exposes state and control, the runner is arizuko's Claude Code container reused unchanged. The LLM is a chatting interface, not a trading automaton." | 2026-10-21 | as F-AST-03/04 | 07, 14 |

## Venues

| ID | Quote | When · where | Requirement | Captured in |
|---|---|---|---|---|
| F-VEN-01 | «make it possible to use it as a standalone terminal.for hyperliquid too... in fact id rather ot be generic but minimal.and simple in that it could be used for other exchsnges.  but mostly is used for rsx and hype tondemonstrate bow... hete you could select exchange by F9 and letter for example» | 2026-07-10 20:41 · f8440a66 | generic venue seam; HL standalone; F9 + letter picker | 13, 10 |
| F-VEN-02 | «... make the terminal work also on https://www.phoenix.trade/» | 2026-07-11 21:17 · b95a6305 | Phoenix read-only venue | 13 |
| F-VEN-03 | Standing rule (commit `bfbd6530`): "Generic multi-venue through the venue seam (`RSX_TERM_VENUE`: rsx, hyperliquid, phoenix); RSX is the first-class venue, the others prove the seam. Offline by default (`RSX_GW_URL=mock`)." | 2026-10-21 | as stated | 13 |

## Honesty and safety

| ID | Quote | When · where | Requirement | Captured in |
|---|---|---|---|---|
| F-HON-01 | «wtf?  this is a demo... sweep and cut all complience and alignment slop» then «I see ok this is not a slop» then «=> no I missunderstood.. what youre doing is good. continue» | 2026-07-08 18:19-18:22 · sb:6fa85434 | THE RULE (last message): the new-trader safety list and "dashed, never faked" stay | 02, 05 |
| F-HON-02 | «but the market maker shiuld quite based on mark price or so.. not synthetic» | 2026-07-10 20:44 · f8440a66 | demo liquidity follows a real mark | 18 |

## Palette and design-system documents

| ID | Quote | When · where | Requirement | Captured in |
|---|---|---|---|---|
| F-DS-01 | «I want you to choose colors similr to a blakck hen cock https://en.wikipedia.org/wiki/Ayam_Cemani ... its black (tinge of green), and blakc-neon-darj-green and violet-dark shades for hilights» | 2026-07-05 19:37 · sb:27e9e5d0 | the Ayam Cemani palette | 02 |
| F-DS-02 | «these colors !... use hem everywhere ,in playground, tui, the casts.» · «make sure the palette is the same as we have for the demos. a unified schema and colors and style» | 2026-07-06 20:34 · sb:53137f6a; 2026-07-08 08:24 · sb:6fa85434 | one palette across every surface | 02 (DS-04) |
| F-DS-03 | «make sure to color individual workds of meaning.. not just whole sentences !....» | 2026-07-06 21:27 · sb:53137f6a | colour the token that carries the meaning, not the line | 02 |
| F-DS-04 | «we keep all the visual elwmnts and design ssytem in detail in the lpcal claide.md and screens.md and visuals.md ... so thst we could rebuild them mostly based on the claude.md salsong and thesw files.  and wed also see and be able to minimize the elements and optimize.» | 2026-07-18 11:16 · b95a6305 | the design system is complete enough to rebuild from; the element inventory supports pruning | 02, 16 |

## Users, access, deployment

| ID | Quote | When · where | Requirement | Captured in |
|---|---|---|---|---|
| F-ACC-01 | «users will just run it on thier computers... no need for an ssh i gues.... ideally also prpvide some containerized version direxlty so they are not afraid of exfiltration etc.» | 2026-07-10 18:18 · f8440a66 | the UI runs on the trader's machine; a container option | 18 |
| F-ACC-02 | «hardcode rsx.krons.cx to tui as the main server and allow a local debug server ... make it user frnedly ismple» | 2026-07-08 10:53 · sb:11825966 | (overridden by F-ACC-03) | 01 |
| F-ACC-03 | Standing rule (commit `bfbd6530`): "Production is explicit: `make term` is disabled on purpose; the hosted deployment is dialled only by an explicit `RSX_GW_URL`." | 2026-10-21 | THE RULE: no default dials the hosted deployment | 12 (GW-08) |
| F-ACC-04 | «the wire proto is protobuf.» · «just mark to support only protobug and quick for the tui !.... nothing else. !» | 2026-07-08 · sb:6fa85434, sb:11825966 | protobuf over QUIC was the stated target; the running gateway speaks JSON over WS (open: OD-06) | 12, 16 |

## Testing, demos, docs

| ID | Quote | When · where | Requirement | Captured in |
|---|---|---|---|---|
| F-TST-01 | «ok for the tui... you need to develop it ahainst tests and jabe integration and play like tests anyway...» | 2026-07-03 10:09 · sb:1b86cc72 | test-first with integration and play tests | 17 |
| F-TST-02 | «create xomprehensive usability tests for the tui... that will show and measure how fast it is ... all its tests need to test the full stack not just the tui... so that any latency is discovered early.» | 2026-07-04 09:17 · sb:27e9e5d0 | full-stack UI tests that also measure speed | 17 |
| F-TST-03 | «can you do a users walkthrough (and also make this into tests) .. ohw are tests done here?  did you clickthrough all flows?  are flows documented?  in FLOWS.md ?» | 2026-07-09 04:20 · sb:6fa85434 | every flow documented and driven by a test | 11, 17 |
| F-TST-04 | «terminal does notseem to work. I suspect it has no play test !» | 2026-07-08 20:19 · cx:019f4235 | no screen ships without a play test | 17 |
| F-DEM-01 | «but i wanted terminal walkthriugh mostly ... dahsboard also good.. but should be ij the pakyground or so.» | 2026-07-11 21:50 · b95a6305 | the trading UI walkthrough is the repo's face; the dashboard demo belongs to the playground | 18 |
| F-DEM-02 | «e.g. some wild move woth liquidations or so and the testing demo trade should just mske you some positive 10k pnl or so» | 2026-07-12 19:24 · b95a6305 | the demo scene: a pump, a liquidation, a winning demo trade | 18 |
| F-DEM-03 | «it shiuld just show tooltips each tooltip shuld be eli13... take from psr.marinade.finance mostly ... its good there. the docs organixation and tooltips.» | 2026-07-20 11:44 · bca65731 | an in-UI walkthrough of ELI13 tooltips reusing the docs | 09, 16 |
| F-DEM-04 | «the demo should be then asciinema recorded and make into a sharable gif so that good viweavle on mobile smakl.screen tall and narrow» | 2026-07-04 13:15 · sb:27e9e5d0 | demo recordings shareable and tall-narrow | 18 |

## Glass technology

| ID | Quote | When · where | Requirement | Captured in |
|---|---|---|---|---|
| F-GL-01 | «tell me if we were to move the term to desktop... which technology offers the lowest latency?  used in video games... ideally inspired sand mimmicing the simpofcitybof the term» | 2026-07-18 10:50 · b95a6305 | glass mimics the terminal's simplicity | 01 |
| F-GL-02 | «qe want to get to 240hz in v2.but not now. but want to pick the right rtech» · «yes do 2. with mailbox. not the complex 240hz stuff» · «its ok to just reender on every data arrival or a a fixed frequency based on the newesr data.» | 2026-07-18 11:06-11:09 · b95a6305 | native GPU window, mailbox present, render the newest state; 240 Hz is later | 19 |
| F-GL-03 | «yes have all on go but have it concentrated all right so we can move it... keep it minimal like the terminal that is important and also record in its clade how we would use the term as a template for it always.» | 2026-07-18 11:40 · b95a6305 | all Go; GPU code behind one seam; minimal; spec how to test and ship the tests | 19 |
| F-GL-04 | «rsx-core is he lin then? if yes its ok...» | 2026-07-19 15:41 · b95a6305 | the shared model is rsx-core, used by every frontend | 19 |
| F-GL-05 | «i think we dont need it now..» (on SDL, sprites, a game theme) | 2026-07-19 16:45 · b95a6305 | no sprites or game theme in v1 | 19 |

## Never stated (searched, nothing found)

No founder statement exists on: vim keys as such; the glass font or atlas;
contrast numbers; whether glass accepts the mouse (the only stance is
F-KEY-05 for the terminal); a visible demo-data label; raw reason numbers;
self-trade fill folding; the microscope or the fisheye by name; `EngineNs`.
These are covered by code behaviour and BUGS entries, and the open ones are in
[16-open-decisions.md](16-open-decisions.md).
