# CEO Report — rsx-term Go Trading Terminal Go-to-Market Audit

Scope: `rsx-term/` (Go Bubble Tea TUI), read against `specs/2/55-terminal.md`
and root `README.md` (latency claims). Read-only. Companion CTO report should
cover production-readiness; this is the sell-it lens.

## 1. The pitch

**"Every other exchange shows you a price. RSX shows you the speed."**
The terminal renders a live ⚡ round-trip strip on every order — net,
internal, and matching-engine legs, broken out — where every other venue's
GUI hides that number behind a spinner or doesn't measure it at all
(`ui/view.go` `viewSpeed`, spec `55-terminal.md` "the terminal's signature: it
*shows* the µs-class path other exchanges hide"). It's terminal-native (SSH,
tmux, a single static Go binary, zero browser/JS), keyboard-only order entry,
and it never fabricates a number — a field with no data source renders `—`,
not a comforting fake zero (`book/latency.go` `NsUnknown`, spec's "Never
fabricate a number" principle). Sell honesty and speed together: a terminal
that tells the truth about how fast it is, instead of a web app that hides
how slow it is.

**Caveat to lead with eyes open (see §4, ranked #1):** today the live wire
only measures the *net* leg — internal/engine legs show `—` in production
until the gateway stamps them (`ui/update.go:92-96`, spec line 328 "not
stamped yet"). The full three-way split is currently a **mock-only** demo
(`conn/mock.go` `DemoScript`). The pitch above is the target state and is
demoable today on the mock — it is not yet what a live desk sees.

## 2. Why a trader/desk buys it

- **Terminal-native.** Low-latency desks already live in tmux/SSH; a Go
  binary that runs there beats another browser tab. `main.go` is a ~150-line
  entrypoint, single static binary, no JS bundle, no Electron.
- **Keyboard-fast order entry.** Full order lifecycle — side, price, qty,
  TIF cycle, reduce-only/post-only toggle, submit, cancel — is single
  keystrokes (`ui/view.go` `helpText`: `b/s side t tif r ro p po tab field
  0-9 type ⌫ del enter submit c cancel`). No mouse, no modal soup.
  Confirm-before-submit exists today (`viewConfirm`, `pendingConfirm`),
  cutting a real class of fat-finger order errors.
- **Latency transparency as a feature, not a debug flag.** The ⚡ strip and
  the F3 trace HUD (`viewTrace`) are first-class UI, not a hidden devtools
  panel. Rolling p50/best over a 128-sample window (`book/latency.go`) means
  a trader watches their own execution quality drift in real time.
  Backed by real bench numbers in the root README (match ~30 ns, ME accept
  266 ns/340 ns, 7.82 µs p50 in-process floor) — this is a rare case where
  the marketing number and the engineering number are the same number.
- **Honest degraded states, not silent hangs.** Book empty or MD link down
  renders an explicit amber "no live book — market-data stream down" row
  (`viewBook`) rather than a stale or blank ladder — a desk trusts a UI that
  tells them when it doesn't know something.
- **Free to try, cheap to run.** No infra beyond a WS endpoint; `RSX_GW_URL=mock`
  runs the entire demo with zero network, zero backend — sales/eval friction
  near zero.

## 3. Demo script (60 seconds, on the mock — always works)

Run: `RSX_GW_URL=mock ./rsx-term` (or `go run .` with the env set).

1. **0-5s — cold open.** Screen fills: three-column layout (book / order
   form / positions+trades), status bar, help line. Say: "This is a Go
   binary, no browser, running over SSH right now."
2. **5-15s — the book streams in.** `feedDemo` paces `DemoScript()` at 30ms
   intervals — bids/asks populate the ladder with colour-coded depth bars,
   two trade prints land in the tape. Say: "Real order book, real fills,
   scripted here so the demo always runs the same way."
3. **15-30s — an order fills.** The scripted own-order lifecycle (accept →
   fill → done) resolves into the positions panel: `LONG +14 @ 9998`,
   uPnL colour-coded green. Say: "That's a real position, derived from this
   session's own fills — labelled `mark=mid` because we don't fake a server
   mark we don't have."
4. **30-50s — the ⚡ strip, the closer.** Point at the speed strip:
   `⚡ RTT 10.44 µs = net 2.5 µs + internal 7.6 µs + engine 0.34 µs · p50
   9.9 µs · best 9.6 µs`. Say: "No other exchange terminal shows you this.
   Binance won't tell you their engine's match latency. We do — because we
   can back it up (cite README: ME match ~30 ns, accept 266 ns)." Hit F3 to
   flip to the trace HUD, showing link status, endpoints, spread, depth —
   "and if you don't believe the headline number, here's the receipt."
5. **50-60s — type an order.** Tab to price, type a few digits, hit enter,
   show the ring-bordered confirm preview (`viewConfirm`) with notional and
   an honestly-labelled `liq — (needs server)` line, esc to cancel. Say:
   "Guardrails first — we show you what we don't know too."

Close on the ⚡ strip. It's the only frame in the whole demo no competitor
can screenshot next to and win.

## 4. What a skeptical buyer/CTO objects to (ranked by deal-impact)

1. **The signature feature isn't live yet.** The internal/engine latency
   split — the actual "wow" of the pitch — only populates on the offline
   mock. On live wire, `viewSpeed` shows `net — + internal — + engine —`
   beyond the total (`ui/update.go:92-96`, honesty table `55-terminal.md`
   line 328: "gateway-stamped — not stamped yet"). A technical buyer who
   asks to see it live gets a degraded version of the exact thing they were
   sold on. **This is the #1 blocker to closing on a live demo** (the mock
   demo is fine for a pitch deck, not for a bake-off).
2. **Cross-process reality is milliseconds, not microseconds.** Root
   `README.md` is explicit: in-process floor is 7.82 µs p50, but
   **cross-process production (GW→ME→GW) is ~1.1 ms** — "~99% of production
   latency is inter-process overhead." The ⚡ strip's µs-class framing, once
   it does go live, needs care in how it's pitched: today's live `net` leg
   alone (client-measured, submit→ack over WS) will likely read in the
   low-to-high hundreds of µs to low ms on a real network, not the ~10 µs
   shown in the mock. Overselling "µs-class" against a demo that turns out
   to be ms-class on a live gateway is a credibility risk, not just a gap.
3. **No auto-reconnect on link loss.** `conn/live.go` `readGw`/`readMd`
   emit `GwDown`/`MdDown` on any read error and simply return — nothing
   redials. A dropped WS mid-session means the trader is dead in the water
   until they restart the process. For a pro desk this is disqualifying on
   its own; it's the kind of thing a CTO tests in the first five minutes.
4. **Positions/margin/liq/funding are Post-MVP or client-derived.** Per the
   spec's own scorecard: liquidation price (✗ missing, MUST-HAVE #1),
   available margin (✗), leverage (✗), funding rate (✗) all show `—` or
   don't exist — blocked on server-side account queries (`A`/`P`/`O`) that
   are explicitly Post-MVP. Position/uPnL shown today are client-side
   derived from the session's own fills plus book-mid, not exchange truth
   (`viewPositions` title literally says `(mark=mid)`). A desk cannot risk-
   manage on this terminal today; it can only watch a book and fire orders.
5. **Single symbol, hardcoded.** `main.go`: `const Symbol = "PENGU-PERP"`.
   No market switcher exists yet (spec's own multi-market vision — options,
   sfdx, lending — is entirely `[needs server]` mockups, not code). Fine for
   a technology demo, a hard no for anyone evaluating multi-asset desk use.
6. **No charts, no order book history, no OI/24h stats.** Spec's own
   nice-to-have list admits this. A pro-desk buyer coming from Bloomberg/TT
   will ask "where's the chart" in the first minute.
7. **JWT/auth maturity.** `main.go` defaults `RSX_GW_JWT_SECRET` to a
   literal `"rsx-dev-secret-not-for-prod-padpad"` if unset — fine for a demo,
   a visible red flag if a technical buyer greps the binary's env defaults.
8. **No mobile / no persistence across restarts.** Terminal-only is a
   feature for the target buyer (§2) but a non-starter for anyone wanting a
   companion mobile app or session continuity — worth stating as an
   intentional non-goal, not a bug, so it doesn't read as an oversight.

## 5. Competitive framing

**vs Binance / Bybit web terminals (retail-scale, browser-based):**
rsx-term does not compete on breadth — no charts, no 200 order types, no
copy-trading, no mobile app. It wins decisively on *honesty and speed
legibility*: no retail exchange terminal shows you engine-level latency, and
none confirms-before-submit with a notional preview by default the way this
does. This is not a retail play; retail wants candles and leaderboards, not
a µs breakdown. Don't pitch retail.

**vs Bloomberg Terminal / TT (Trading Technologies) / pro-desk incumbents:**
This is the right comparison class — terminal-native, keyboard-driven,
built for someone who lives at a desk all day. rsx-term wins on radical
simplicity (a 30-file Go program vs. a decades-old integrated platform),
zero-cost trial (`RSX_GW_URL=mock`, no license, no sales call), and — once
§4.1/§4.2 are closed — a genuinely differentiated latency-transparency
feature neither Bloomberg nor TT offers (they don't expose their own
network/engine timing to the trader at all). It loses badly on breadth
today: no multi-asset, no risk desk tooling (margin/liq/funding all
missing), no historical data, no scripting/API surface comparable to
Bloomberg's. **Positioning: not "replace your Bloomberg terminal," but
"the fastest, most honest single-market perps terminal that exists" — a
wedge, not yet a platform.**

## 6. Verdict

**Demo-ready today, on the mock, for the right room.** The 60-second script
in §3 is genuinely strong — it's the only exchange demo in existence that
closes on an engine latency number instead of a UI animation, and every
number on screen during that demo is either real or honestly labelled `—`.
It is **not** ready for a technical bake-off against a live gateway, and
should not be pitched with live µs-latency claims until the gap in §4.1/§4.2
is closed — a sharp technical buyer will ask to see it live within the first
five minutes and the live strip currently degrades to `net — internal —
engine —` beyond the total.

**Top 3 to build/fix before this headlines a sales demo:**

1. **Stamp internal/engine latency on the live wire** (gateway-side
   timestamps per webproto-49) so the ⚡ strip's full split is real in
   production, not just in `DemoScript()`. This is the single highest-
   leverage fix — it turns the pitch's centerpiece from "mocked" to "true."
2. **Auto-reconnect on GW/MD link loss** (`conn/live.go` currently dead-ends
   on any read error). A live demo that dies on one dropped packet loses
   the room instantly; this is table stakes before any live bake-off.
3. **Decide and rehearse the honest latency number for the live pitch.**
   Either demo over localhost/LAN where the net leg is genuinely small, or
   explicitly reframe the pitch around the *transparency* (showing the
   true number, whatever it is) rather than a specific µs figure — don't
   let "µs-class" survive contact with a live cross-process ~1.1 ms reality
   the README itself documents.

Everything else in §4 (positions/margin/liq, single-symbol, no charts) is
honestly labelled as Post-MVP in the terminal's own spec and does not need
to be hidden — the "we tell you what we don't know" framing turns those gaps
into supporting evidence for the pitch rather than embarrassments, *provided*
the latency headline itself is airtight first.
