# rsx-term — one page

**The whole order book, and its history, as a live heatmap in your terminal.**
Keyboard-only. Runs anywhere, over SSH. No mouse, no GPU, no charts — just order
flow you can *see*.

---

### The idea

Pro traders read *order flow* — where liquidity rests, where it's pulled (spoofs),
where trades eat it (absorption). The tool for that is Bookmap: a heavy, closed,
GPU heatmap in its own window. **rsx-term does it in text.** A live liquidity
heatmap where **time flows up** on a logarithmic cadence (live → 10 s → minutes →
hours), **price** runs on a fisheye (fine at the touch, compressed deep), and each
cell's colour + glyph carry resting size and order count. Trades overlay as a
second layer. Walls appearing and vanishing, trades absorbed or breaking through —
they become shapes you recognise, not stats you compute after the fact.

### Why it's different

- **It's text.** A character grid, not a bitmap — so it runs over SSH, at trivial
  cost, and the same compressed model can drive a bitmap frontend later unchanged.
- **It's honest.** It never shows a number it can't stand behind: unknowns dash,
  estimates are marked `~`, fat-fingers are hard-blocked, overflowing P&L is
  withheld — not wrapped.
- **It's keyboard-fast.** Size on `1`–`5`, aggressive on shift, one key to place /
  one to cancel, a price cursor on `h`/`l`. A mouse would be a fat-finger liability;
  there isn't one.
- **It's generic multi-exchange.** RSX for trading, read-only Hyperliquid for real
  breadth; a new venue is one adapter file.

### Three screens, one key apart

| Screen | For |
|---|---|
| **BOOK** | the heatmap — hand market-making on one symbol; hop symbols by letter-code; freeze a moment and hand it to the assistant |
| **NEWS** | market overview — sector map + news feed + which symbols are moving together |
| **LLM** | an assistant that receives the frozen book window or a headline as context |

### The engineering, in one line

Three nonlinear compressions (log-time cadence, price fisheye, log-size colour)
fit a whole book + history into a fixed grid; the glyph vocabulary is
*empirically calibrated* (rasterised and coverage-ranked, not guessed); the state
folds are pure and clock-free, so the render is a pure function of the model.

### See it

```sh
RSX_TERM_STREAM=1 make term-demo      # offline, no setup — press ? for help
```

### Positioning

Not an HFT tool (a human can't beat µs latency — that's the API), not a charting
package, not a Bloomberg. A focused, honest, keyboard-only surface for
discretionary order-flow reading and manual execution — and a genuinely novel
demo of what a terminal can do.

*Deeper: `README.md` (run it) · `ARCHITECTURE.md` (how) · `notes/` (why).*
