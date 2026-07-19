# Element inventory — the minimize / optimize prune list

Every distinct visual element, glyph, colour, and ramp rsx-term can draw, tagged
so the founder can prune the set before the GPU/desktop rebuild. **Record-only**
— nothing here edits the render; it is the decision surface. The full language
is `VISUALS.md`; per-screen placement is `SCREENS.md`. Each row cites
`file:symbol`.

Tag meanings:

- **essential** — a distinct data channel or honesty marker; keep on any
  renderer.
- **redundant** — duplicates a meaning already carried elsewhere; a drop
  candidate.
- **mergeable** — two-or-more glyphs/elements doing one job; unify into one.
- **cuttable** — a terminal-cell *workaround* the GPU layer makes obsolete (real
  rectangles, real alpha, always-full-colour), or a legacy surface the heatmap
  supersedes.

The GPU lens: a desktop renderer has true rectangles, sub-pixel bars, real alpha
and unlimited colour. Much of rsx-term's glyph vocabulary exists *because a
character cell has none of those*. Those glyphs encode channels that are
essential, but their **encoding** is cuttable — the channel survives as a
primitive, the glyph does not.

## Colours (`ui/styles.go`)

| Colour | Meaning | Tag | Rationale |
|---|---|---|---|
| `ColorLive`/`ColorBid` `#22f5a1` | live/long/bid/filled/positive | essential | the one positive semantic + the size-ramp ceiling |
| `ColorAsk` `#f87171` | short/ask/down/reject/negative | essential | the one negative semantic |
| `ColorHeading` `#bd83ff` | badge / ⚡ / section titles | essential | primary accent; distinct from `ColorAccent`'s role |
| `ColorAccent` `#a992ff` | own-order / sparkline / mode-tag / keycast | mergeable | one hue, several roles; a GPU could fold badge+accent to one violet |
| `ColorRing` `#7c3aed` | overlay ring (confirm/trace/help) | mergeable | a third violet; could share `ColorHeading`/`ColorAccent` if borders differ by weight |
| `ColorText` `#a9bcb2` | body text | essential | base readable ink |
| `ColorTextBright` `#e7eeea` | focused / active / bright | essential | the focus/attention step above body |
| `ColorMuted` `#586b62` | labels / captions / idle | essential | the "secondary" step; carries `StyleDerived` too |
| `ColorDegraded` `#fbbf24` | stale / offline / degraded | essential | the one warning semantic |
| `ColorPageBg` `#040806` | page bg + size-ramp floor | essential | ramp floor is load-bearing |
| `ColorPanelBg` `#0d1712` | panel bg | redundant | barely distinct from page bg; a GPU flat theme may not need a separate panel fill |
| `ColorBorder` `#16211b` | panel border | cuttable | box-drawing borders are a TUI frame; GPU panels use spacing/elevation |
| colorblind `#2f9bff`/`#ff9e3d` | bid/ask swap | essential | accessibility; keep the swap seam |

Three violets (`Heading`/`Accent`/`Ring`) is the clearest colour-merge
opportunity: they separate by *role*, not by three genuinely-different meanings.

## Streaming glyph table (`ui/stream.go` `glyphs`)

| Glyph(s) | Role | Tag | Rationale |
|---|---|---|---|
| `countRamp ░▒▓█` | order-count density | cuttable (channel essential) | count is a real channel, but shade-block *texture* is a cell hack; GPU draws count as e.g. stipple/number over real alpha |
| `microRamp ▁▂▃▄▅▆▇█` | NOW-row live depth bars | cuttable | bottom-weighted cell bars; GPU draws a real histogram |
| `tradeBuyRamp ▵▴△▲` / `tradeSellRamp ▿▾▽▼` | trade magnitude + side-by-shape, aggressor-hued | essential | the co-equal trade layer; mirrored ladders keep it off the book AND carry side without hue — keep as marker sizes/polarity |
| `newsRamp ·►►‼` | news severity (4 tiers) | redundant | `►` repeats at tiers 1 & 2 (hue separates them) — only 3 distinct runes for 4 tiers; pick a 4th or go hue-only |
| `persistent ▚` | ≥30 s liquidity | essential | the one "long-standing" mark; a real data channel |
| `ownOrder ◇` | your order on the map | mergeable | see own-order note below — 3 glyphs for one concept |
| `ownBuy ▲` / `ownSell ▼` | your order on the ruler | mergeable | side already reads from ruler position; two more own-order glyphs |
| `cursor ┃` | price cursor | essential | the keyboard-only interaction anchor |
| `touchTick ┼` | the two touch columns | essential | marks the spread on the ruler |
| `rulerLine ─` | fisheye baseline | cuttable | a drawn axis line; GPU renders an axis primitive |
| `railIdle │` | idle news rail | redundant | a blank column would read the same; it only fills space |

**Own-order over-encoding.** "Your resting order" is drawn three ways: `◇` on
the heatmap, `▲`/`▼` on the ruler, `▸` on the DOM ladder — plus the accent hue
each time. A GPU layer wants **one** own-order treatment (e.g. an outlined cell
in accent), collapsing four glyphs to one primitive. Highest-value merge.

## Fixed / chrome glyphs

| Glyph | Where | Tag | Rationale |
|---|---|---|---|
| `▸` | microscope row · news/DOM cursor · DOM own-order | mergeable | one glyph, four roles; the own-order role overlaps `◇` |
| `┆` | BOOK tape-rail separator | cuttable | a column rule; GPU uses spacing |
| `▊` | DOM depth-bar cell (`depthGlyph`) | cuttable | cell bar → real rectangle |
| `█` | DOM imbalance bar · latency `hopBar` | cuttable | solid cell fill → real rectangle |
| `‹` | DOM last-print row | essential (channel) | "last trade here" is real; glyph may become a row highlight |
| `≡ = ≠ ·` | NEWS co-move (`coMoveGlyph`) | essential | 4-way coupling channel; `≠` doubles for two negative tiers (weight separates) — minor redundancy |
| `●` link dot | header / status bar | essential | link up/down at a glance |
| `◀ ▶` | BOOK price-axis legend | redundant | "bids/asks" text already labels the sides |
| `⚡` | speed strip | essential | the speed motif; keep as an icon |
| `⚠` | DOM ARMED banner | essential | the guardrail-down warning |
| `⏳` | LLM in-flight | essential | honest "waiting", not fake content |
| `⌨ ␣ ↑↓←→` | keycast chip | cuttable | demo/teaching aid; a desktop app has OS-level keycast |
| `░░░░░░░░` | DOM risk row | essential (marker) | dashed unknown-margin bar; honesty marker, keep as an empty meter |
| `~` | `~mark` / `~uPnL` / window labels | essential | the estimate marker — core honesty tell |
| `B / S` | DOM tape side | redundant | colour + geometry already carry side (kept for colourless mode) |
| `_` | focused field / chat caret | essential | text cursor |
| `── …` | section dividers / overflow | cuttable | rules → GPU spacing; `…` overflow → real truncation/scroll |

## Ramps, bars, and scales

| Element | Role | Tag | Rationale |
|---|---|---|---|
| size ramp `rampColor` (log, 4 tiers) | resting size → background | essential | the heatmap's core encoding — the ONE ramp |
| count tiers (`countTier`, 5 steps) | order count → shade glyph | essential (channel) | second data channel; encoding cuttable (see shade blocks) |
| trade tiers (`tradeTier`, log 4) | print size → `▵▴△▲`/`▿▾▽▼` | essential | trade-layer magnitude (per-side ladders) |
| micro-bar (linear, 8 steps) | NOW-row exact depth | mergeable | linear-to-`refMax`; overlaps the size ramp's job on one row — GPU unifies |
| move-diverging (`moveTiers` 10/50/200/800 bp) | sector tile colour | essential | the ONE diverging scale (only red↔green place); keep, note the exception |
| `sparkRamp ▁▂▃▄▅▆▇█` (`ui/sparkline.go`) | RTT sparkline | mergeable | **same 8 runes as `microRamp`, declared twice** — collapse to one eighth-block constant |
| `hopBar` (3-seg proportional) | latency legs | cuttable | cell bar → real stacked bar |
| `imbalanceBar` (2-seg, 20 wide) | bid/ask depth share | cuttable | cell bar → real bar |
| `depthBar` (`▊`, 6 wide/side) | DOM per-level depth | cuttable | cell bar → real bar |
| stable basis (`foldBasis`, rise-fast/decay-slow) | ramp reference | essential | anti-flicker; a real algorithm, survives the port |

## Layout regions & overlays

| Region | Tag | Rationale |
|---|---|---|
| BOOK header / mode line / ruler / footer | essential | the streaming grid's frame — the primary surface to rebuild |
| news rail (col 0) | essential | severity channel inline with time |
| heatmap body (far/live/NOW rows) | essential | the product |
| log-time gutter (`−10m`…`now`) | essential | the time-axis labels |
| tape rail (right) | mergeable | duplicates map trades as a feed; keep OR fold into a hover/side panel on GPU |
| touch ladders (ask/bid, 3 levels) | essential | the only exact numbers on the BOOK screen |
| position / PnL line | essential | the trader's own state |
| latency ⚡ line | essential | the live-speed tell |
| hint line (keymap-generated) | essential | the k9s "what can I press now" |
| microscope status line | essential | the honest freeze-window label |
| keycast chip | cuttable | demo aid (see glyph row) |
| NEWS sector map / co-move / feed | essential | the market-context screen |
| LLM context block / reply pane | essential | the handoff contract, made visible |
| **classic DOM view** (whole `ui/view.go` tree) | cuttable | the golden-locked *legacy default*; the heatmap supersedes it — the GPU successor need not port the three-column DOM panel |
| trace HUD (F3) | mergeable | a superset of the speed strip; on GPU one expandable latency panel covers both |
| confirm preview / help overlay / armed banner | essential | the safety + discoverability surfaces |
| degradation ladder (`detectMode`: true/shade/plain) | cuttable | a GPU renderer is always full-colour; `modeShade`/`modePlain` + `detectMode` are terminal-only |
| colourblind theme swap | essential | accessibility; keep the seam, not the degradation tiers |

## Summary

Roughly **60 distinct elements** across colours, glyphs, ramps, and regions.
Tally (elements can carry a channel-vs-encoding split; counted by primary tag):

- **essential ≈ 33** — the data channels (size ramp, count, trade layer,
  persistence, co-move, news severity, move-diverging), the honesty markers
  (`—` `~` dim armed `░░░░` `⏳`), the interaction anchors (cursor, touch ticks,
  ladders), and the region frames.
- **cuttable ≈ 15** — everything that exists because a character cell lacks real
  rectangles/alpha/colour: the cell bars (`▊` `█` micro/hop/imbalance/depth), the
  shade-block *texture*, box-drawing frames/rules, the whole degradation ladder,
  and the legacy DOM view tree.
- **mergeable ≈ 8** — the own-order trio+`▸` (biggest win: 4 glyphs → 1), the
  three violets, `microRamp`/`sparkRamp` (one eighth-block constant), the trace
  HUD vs speed strip, the tape rail vs map trades.
- **redundant ≈ 4** — `newsRamp`'s repeated `►`, `railIdle │`, the `◀ ▶` axis
  arrows, the `B`/`S` tape glyphs (colourless-mode only).

The notable, high-value cuts/merges for the GPU rebuild:

1. **Collapse the four own-order glyphs (`◇` `▲` `▼` `▸`) to one accent
   treatment** — one concept currently drawn four ways.
2. **Drop the cell-bar glyph family** (`▊` `█`, the eighth-block bars, hop/
   imbalance/depth) for real rectangles — the largest cuttable group.
3. **Retire the truecolour→256→16→plain degradation ladder** (`detectMode`,
   `modeShade`, `modePlain`) — a GPU is always full-colour; keep only the
   colourblind swap.
4. **Merge `microRamp` and `sparkRamp`** — literally the same eight runes twice.
5. **Fold the three violets** (`Heading`/`Accent`/`Ring`) toward fewer hues
   separated by weight/elevation rather than three near-identical colours.
6. **Decide the DOM view's fate** — it is a golden-locked legacy surface the
   heatmap supersedes; the successor likely ports only the streaming tree.
