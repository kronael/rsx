#!/usr/bin/env bash
# Driven rsx-term walkthrough — recorded by asciinema (see this dir's Makefile,
# `make gif`; render details in CLAUDE.md). Boots the terminal in a tmux pane
# on the LIVE RSX cluster staged by the demo scene driver
# (`./rsx-playground/playground demo-scene prep` — PENGU-only cluster, maker
# quoting the "walls" depth profile, a leveraged short victim staged), then
# STARTS the scene's `run` stage itself so the recording captures the whole
# arc. The KEYCAST overlay is on: every keystroke shows as a bottom-right chip.
#
#   1 BOOK    walls-profile heatmap: thin touch, deep walls — a real depth
#             gradient; the calm market breathes (maker jitter + taker prints)
#   2 ENTRY   the demo user's four real IOC buy clips print on the tape;
#             the position line flips to LONG (fills fold it, nothing faked)
#   3 PUMP    the scene runs PENGU up +45% — the ladder marches across the
#             map, and the RISK ENGINE really liquidates the staged short
#             (a 300k-token forced buy prints mid-pump); resting limit
#             placed + cancelled while it runs (b, j, f, d; r/p toggles)
#   4 SCOPE   ↑↑↑ microscope over the move's history rows (honest aggregate
#             labels) · enter freezes the row → assistant handoff
#   5 NEWS    sector tiles + cross-symbol co-movement + / search
#   6 LLM     the frozen handoff's REAL streamed arizuko reply, then a typed
#             question → second real reply
#   7 VENUES  F9 picker → hyperliquid (real BTC book, read-only) → back ·
#             ? keymap-generated help
#   8 BOOK    the money shot: position line ~+10k unrealized (real fills vs
#             the live mid), time-cascade bands showing the pump's history
#
# Prereqs (the Makefile's `rec` checks the gateway):
#   - Scene cluster staged FRESH: `./rsx-playground/playground demo-scene prep`
#     from the repo root (books live, victim short staged). Record promptly —
#     the demo cluster's event path degrades with uptime (BUGS.md
#     RISK-CMP-DESYNC-ORDER-EVENT-STARVE).
#   - RT_BIN points at a prebuilt rsx-term binary (Makefile builds it).
#   - RSX_TERM_ASSIST set to a live arizuko /chat/{token} URL for the real
#     assistant round-trip (mint: .ship/45-ARIZUKO-LLM/chat-token.sh). Unset,
#     the pane honestly shows the offline placeholder and the walk still runs.
#
# The driver polls the pane (tmux capture-pane into $(..) — never stdout, the
# recording must stay clean) so steps key off what is actually on screen.
set -u
SESS=rtwalk
BIN="${RT_BIN:-/tmp/rt-walk}"
VENUE="${RT_VENUE:-rsx,hyperliquid}" # primary rsx (trading); HL for the F9 hop
USER_ID="${RT_USER:-1}" # the scene's winner (playground-seeded; 0 has no collateral)
WATCH="10:PENGU-PERP:p" # the scene's featured symbol (PENGU-only cluster)
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

tmux kill-session -t "$SESS" 2>/dev/null || true
tmux new-session -d -s "$SESS" -x 118 -y 33
tmux set-option -t "$SESS" status off # hide tmux chrome — record just the terminal
# RSX_TUI_*_DECIMALS: the playground gateway advertises 8/8 for every symbol
# (config.rs boot default — BUGS.md); 6/4 are PENGU's real decimals per
# rsx-playground/runtime.py, so the display (and the position line's PnL
# scaling) is honest.
tmux send-keys -t "$SESS" "RSX_TUI_USER=$USER_ID RSX_TERM_VENUE=$VENUE RSX_TERM_STREAM=1 RSX_TERM_NEWS=1 RSX_TERM_KEYCAST=1 RSX_TUI_PRICE_DECIMALS=6 RSX_TUI_QTY_DECIMALS=4 RSX_TERM_WATCH='$WATCH' RSX_TERM_ASSIST='${RSX_TERM_ASSIST:-}' $BIN" Enter

# snap captures the pane text (via command substitution only — nothing may
# leak into the recorded pty).
snap() { tmux capture-pane -t "$SESS" -p 2>/dev/null; }

# wait_for <grep-pattern> <timeout-s>: poll the pane until it matches.
wait_for() {
  local pat=$1 deadline=$((SECONDS + $2))
  until snap | grep -q "$pat"; do
    ((SECONDS >= deadline)) && return 1
    sleep 1
  done
}

# wait_reply <timeout-s>: on the LLM screen, wait until the in-flight turn
# lands — the ⏳ busy marker leaves the status line when the reply folds in.
wait_reply() {
  local deadline=$((SECONDS + $1))
  while snap | grep -q '⏳'; do
    ((SECONDS >= deadline)) && return 1
    sleep 2
  done
}

# type_text <string>: send literal chars one by one (a single send-keys -l
# arrives as one bubbletea key event and gets dropped; per-char also reads
# as real typing in the recording). "--" so a "-" char isn't an option.
type_text() {
  local s=$1 i
  for ((i = 0; i < ${#s}; i++)); do
    tmux send-keys -t "$SESS" -l -- "${s:$i:1}"
    sleep 0.04
  done
}

k() { tmux send-keys -t "$SESS" "$@"; }

{
  # ── 1 · BOOK: the walls-profile heatmap paints its gradient ────────────
  wait_for 'mid 0\.' 30 # live book: a real mid on the header
  sleep 6               # calm bins fold: thin touch, deep walls, taker prints

  # Start the scene timeline (calm 12s / pump 25s / retrace 15s / drift).
  # Scene t=0 is here; the sleeps below track it.
  (cd "$REPO_ROOT" && setsid ./rsx-playground/playground demo-scene run \
    >tmp/rt-walk-scene.log 2>&1 &) 2>/dev/null

  # ── 2 · calm: cursor keys while the winner's REAL entry clips print ───
  k j; sleep 1.5                 # cursor → best bid (scene t≈1.5)
  k l; sleep 1; k l; sleep 1     # two ticks up
  k k; sleep 1.5                 # snap to the ask touch (t≈5, clips printing)
  sleep 4                        # entry fills fold → position line goes LONG
  k h; sleep 1                   # one tick back (t≈10)

  # ── 3 · PUMP: stay on BOOK — the touch marches, the victim liquidates ─
  sleep 4                        # t≈14: the run-up starts to move the map
  k k; sleep 2.5                 # follow the marching ask (t≈16.5)
  k b; sleep 1                   # side: buy (t≈19)
  k j; sleep 1                   # join the best bid
  k f; sleep 3                   # resting limit ◇ — liquidation lands ~here
  k d; sleep 2                   # cancel it (t≈26)
  k k; sleep 2                   # snap ask — the price is far above entry now
  k r; sleep 1; k r; sleep 0.7   # reduce-only on → off (mode line blips)
  k p; sleep 1; k p; sleep 0.7   # post-only on → off (t≈33)
  sleep 4                        # ride the top of the pump

  # ── 4 · retrace: microscope over the move's history, freeze → assistant
  sleep 3                        # t≈40: retrace under way
  k Up; sleep 1; k Up; sleep 1; k Up; sleep 1.5 # older rows, aggregate labels
  k Enter # freeze the row → hands book context to the assistant
  wait_for 'context handed off' 10
  sleep 4 # read the frozen context block (reply cooks in the background)
  k Tab; sleep 1.2    # LLM → back to BOOK
  k Escape; sleep 1   # microscope off — back on the live hint line (t≈53)

  # ── 5 · NEWS: sector tiles + co-movement + search ──────────────────────
  k n; sleep 4.5
  k /; sleep 0.7; type_text "btc"; sleep 2.2; k Escape; sleep 1

  # ── 6 · ASSISTANT: the real streamed reply + a typed question ─────────
  k Tab; sleep 1.5 # news → assistant pane
  wait_reply 90    # the freeze turn's reply (real arizuko round-trip)
  sleep 5          # read it
  type_text "a short just got liquidated into this pump. would you chase here or wait for the retrace?"
  sleep 0.8; k Enter; sleep 1.5
  wait_reply 150 # honest ⏳ wait — a fresh Claude container per turn
  sleep 6.5      # read the second reply

  # ── 7 · venues + help ──────────────────────────────────────────────────
  k Tab; sleep 1           # leave the chat pane (typing captures ? and q there)
  k F9; sleep 2.2          # picker on the mode line: r rsx · h hyperliquid
  k h; sleep 4             # → live HL BTC book (real market, read-only)
  k F9; sleep 0.8; k r; sleep 1.5 # back to rsx
  k '?'; sleep 4.5         # keymap-generated help overlay
  k Escape; sleep 1

  # ── 8 · the money shot: LONG position ~+10k over the pump's history ───
  k j; sleep 1.5           # cursor to the live touch
  sleep 8                  # hold: pos line ~+10k, cascade bands, walls ramp
  k q; sleep 1
  pkill -f "demo_scenario.py run" 2>/dev/null || true
  tmux kill-session -t "$SESS" 2>/dev/null || true
} >/dev/null 2>&1 &
drv=$!
tmux attach -t "$SESS"
wait "$drv" 2>/dev/null || true
