# Keyboard Driven Terminal

`rsx-tui` is a ratatui trading terminal because order entry is
an input-bandwidth problem. A keyboard turns actions into
home-row chords and hotkeys; a mouse turns them into serial
point-and-click targeting.

## Input space

Input space is actions reachable per second plus the muscle
memory that makes those actions reliable under stress. In the
terminal, digits edit price and qty, `Tab` changes field, `b`
and `s` choose side, `t` cycles TIF, `Enter` submits, `F3`
toggles diagnostics, and `q` / `Esc` exits. Those 9 action
classes are reachable without moving the hand to a pointer.

A mouse is lower bandwidth because it is serial: acquire one
visual target, move, click, then acquire the next target. Fitts's
law is the cost model: time rises with target distance and falls
with target size. A dense trading GUI is therefore slowest
exactly where it matters most — small adjacent controls under
time pressure.

## Why terminals work for traders

The prior art is not decorative. Bloomberg terminals and
vim/modal editors both win by making repeated actions addressable
from keys, not by hiding them behind pointer targets. The user
pays the learning cost once, then execution becomes recall
instead of search.

That shape fits RSX. The terminal is one screen for one
instrument: ladder, order form, positions, tape, latency strip,
and an always-visible key-hint bar. The book can move; the
submit path does not depend on chasing a moving button.

## What you give up

Keyboard-first UIs are less discoverable. A new user does not
guess every chord, and modal mistakes can be expensive in a
trading screen. RSX mitigates that by keeping the key-hint bar
visible at all times and by keeping the built command set small:
the current live path is those 9 action classes, not a hidden
hundred-command surface.

The tradeoff is acceptable because the terminal is a power-user
surface, not the only client. The mouse GUI can optimize first-
time comprehension; the terminal optimizes repeated execution.

---

Deeper: [specs/2/55-terminal.md](../../specs/2/55-terminal.md),
[specs/2/54-tui-access.md](../../specs/2/54-tui-access.md),
[specs/2/49-webproto.md](../../specs/2/49-webproto.md),
[blog/25-trade-ui-notes.md](../../blog/25-trade-ui-notes.md)
