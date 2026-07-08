# 42 — rsx-term audits synthesis + punch-list

Two audits of the Go terminal (`rsx-term`): `13YO-REPORT.md` (new-trader
usability) and `CEO-REPORT.md` (sell-as-best-GUI). Both agreed on the shape:
the terminal is **further along than the spec claims** (confirm-preview and
LONG/SHORT labels already ship), the *market-data + execution* half is solid,
and the gaps split cleanly into **cheap real code fixes** vs **honestly-absent
Post-MVP server data**. This is the prioritized close-list.

## Tier 1 — cheap code fixes (do now; "simple/fast/usable" wins)

1. **Confirm-flow friction** (`ui/update.go:196-224`, `ui/view.go:178-187`) —
   confirm-before-submit exists, but two fast `enter`s fire with no friction
   and the hint `"enter → confirm"` doesn't explain the two-stage flow. Fix:
   make the confirm an explicit distinct action (e.g. the preview says
   `enter again to SEND · esc cancel`), and don't let a held/double enter
   skip the preview. The #1 fat-finger guard — make it legible.
2. **Derived-vs-authoritative visual differentiation** (`ui/view.go:70-74,
   198-234`) — `mark=mid` and uPnL are client-computed from the local book
   mid but render with the same confidence as real data. Label-by-name isn't
   enough. Fix: render derived values dim/italic with a `~` prefix (`~mark`,
   `~uPnL`) so a beginner can't mistake the estimate for exchange truth. The
   single most dangerous mislabeling in the terminal.
3. **uPnL "—" needs a reason** (`ui/view.go:215-225`) — when `book.Mid()`
   has no book it silently dashes; mirror the explicit amber "no live book"
   caption used elsewhere (`ui/view.go:24`).
4. **Sharpen the liq placeholder** (`ui/view.go:183`) — `"liq — (needs
   server)"` reads like a debug string. Make it read as a deliberate
   not-yet: `"liq  n/a"` with a one-line legend, not a raw TODO.
5. **Expand tif/ro/po abbreviations** in the order form — spell them on
   first sight (`GTC`, `reduce-only`, `post-only`), keys stay short.
6. **Trade tape B/S text** — the tape is color-only; add a `B`/`S` glyph so
   it's readable without relying on color (accessibility + clarity).

## Tier 2 — medium code (demo-readiness)

7. **Auto-reconnect on link loss** (`conn/live.go`) — a socket drop is
   currently terminal for that reader (`GwDown`/`MdDown` end the goroutine).
   Both audits flag it: a demo/desk terminal must survive a blip. Add
   bounded backoff reconnect per socket; md reconnect re-sends `{S:...}`.
8. **Close-position action** (no close exists anywhere) — add a
   close/flatten key that submits a reduce-only order against the derived
   position. Pairs with reduce-only-default-on-close (the new-trader guard).

## Tier 3 — Post-MVP, server-blocked (label honestly, do NOT fake)

All trace to one root cause: no margin/leverage/balance/mark-price wire data
(`wire/`), and `O`/`P`/`A` account queries are Post-MVP (`specs/2/49`):
liquidation price, leverage, available margin, ROE%, margin-ratio bar,
size-as-%-balance, funding rate + countdown, true exchange mark price. Keep
them dashed/absent with honest legends — the terminal's integrity is that it
never fabricates these.

## The CEO truth-gap (messaging + one server ask)

The ⚡ speed strip's net/internal/engine split is only *fully* real on the
mock; live wire populates only the net leg (internal/engine `—` until the
**gateway stamps them**), and production cross-process RTT is ~1.1 ms vs the
7.82 µs in-process floor the mock shows. So the "we show the µs latency others
hide" pitch is true in the demo, not yet live. Two fixes: (a) gateway stamps
the internal/engine legs into the live frames (server work, not terminal);
(b) the sales narrative quotes an honest live number, not the in-process
floor. Until (a) lands, the demo runs on the mock and says so.

## Spec debt

`specs/2/55-terminal.md:171-205` "New-trader requirements & current scorecard"
is stale against `rsx-term` — it lists confirm-preview and LONG/SHORT as
missing/partial when both ship. Update the scorecard to score the Go terminal.
