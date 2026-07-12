#!/usr/bin/env bash
# Driven rsx-term walkthrough — recorded by asciinema (see this dir's Makefile,
# `make gif`; render details in CLAUDE.md). Boots the terminal in a tmux pane
# on the LIVE RSX cluster (trading enabled, Hyperliquid riding read-only for
# the venue picker), with the KEYCAST overlay on so every keystroke shows as
# a bottom-right chip labelled from the keymap. The full feature tour:
#
#   1 BOOK    heatmap streams · real fills paint the trade layer + tape rail
#   2 CURSOR  h/l one tick · j/k snap to the best bid/ask touch
#   3 SCOPE   ↑↑↑ microscope over older rows (honest aggregate labels) ·
#             enter freezes the row → assistant (reply cooks in background)
#   4 TRADE   b side · f resting limit at the cursor (◇ rests on the map,
#             ▲ on the ruler) · d cancel · r/p mode toggles ·
#             x + code symbol hop PENGU → SOL → home
#             (no IOC cross: the demo cluster wedges its completion path —
#             BUGS.md RISK-CMP-DESYNC-ORDER-EVENT-STARVE — place/cancel is
#             the reliable window)
#   5 NEWS    sector tiles + cross-symbol co-movement + / search
#   6 LLM     the frozen handoff's REAL streamed arizuko reply, then a typed
#             question → second real reply
#   7 VENUES  F9 picker list → hyperliquid (real BTC book) → back ·
#             ? keymap-generated help · q quit
#
# Prereqs (the Makefile's `rec` checks the first two):
#   - RSX cluster up with live books: `./rsx-playground/playground reset --yes
#     && make demo` from the repo root (3 tokens quoting).
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
USER_ID="${RT_USER:-1}" # playground-seeded demo user (0 has no collateral)
WATCH="10:PENGU-PERP:p,3:SOL-PERP:s,1:BTC-PERP:b"

tmux kill-session -t "$SESS" 2>/dev/null || true
tmux new-session -d -s "$SESS" -x 118 -y 33
tmux set-option -t "$SESS" status off # hide tmux chrome — record just the terminal
tmux send-keys -t "$SESS" "RSX_TUI_USER=$USER_ID RSX_TERM_VENUE=$VENUE RSX_TERM_STREAM=1 RSX_TERM_NEWS=1 RSX_TERM_KEYCAST=1 RSX_TERM_WATCH='$WATCH' RSX_TERM_ASSIST='${RSX_TERM_ASSIST:-}' $BIN" Enter

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
# (Counting "asst" tag rows breaks: a long reply scrolls the older tag off.)
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
  # ── 1 · BOOK: heatmap streams, the trade layer paints ─────────────────
  wait_for 'mid 0\.' 30 # live book: a real mid on the header
  sleep 6               # real maker/taker fills build rows + the tape rail

  # ── 2 · price cursor: one-tick steps, touch snaps ──────────────────────
  k j; sleep 1.4                # cursor → best bid (keycast: chip bottom-right)
  k l; sleep 0.9; k l; sleep 0.9 # two ticks up
  k h; sleep 0.9                # one tick back down
  k k; sleep 1.4                # cursor → best ask

  # ── 3 · microscope: older rows (honest labels), freeze → assistant ────
  k Up; sleep 1; k Up; sleep 1; k Up; sleep 1.5 # older rows, aggregate labels
  k Enter # freeze the row → hands book context to the assistant
  wait_for 'context handed off' 10
  sleep 4 # read the frozen context block (reply cooks in the background)

  # ── 4 · trading: place → rest → cancel · toggles · symbol hop ─────────
  k Tab; sleep 1.2    # LLM → back to BOOK
  k Escape; sleep 0.8 # microscope off — back on the live hint line
  k b; sleep 1        # side: buy
  k j; sleep 1        # join the best bid
  k f; sleep 3.2      # place resting limit → ◇ on the map, ▲ on the ruler
  k d; sleep 2.4      # cancel it (nearest the cursor)
  k r; sleep 1.1; k r; sleep 0.7 # reduce-only on → off (mode line blips)
  k p; sleep 1.1; k p; sleep 0.7 # post-only on → off
  k x; sleep 0.9; k s; sleep 2.6 # symbol hop → SOL-PERP live book
  k x; sleep 0.7; k p; sleep 1.6 # home to PENGU-PERP

  # ── 5 · NEWS: sector tiles + co-movement + search ──────────────────────
  k n; sleep 4.5
  k /; sleep 0.7; type_text "sol"; sleep 2.2; k Escape; sleep 1

  # ── 6 · ASSISTANT: the real streamed reply + a typed question ─────────
  k Tab; sleep 1.5 # news → assistant pane
  wait_reply 90    # the freeze turn's reply (real arizuko round-trip)
  sleep 5          # read it
  type_text "would you join the bid or lift the ask into this book? why?"
  sleep 0.8; k Enter; sleep 1.5
  wait_reply 150 # honest ⏳ wait — a fresh Claude container per turn
  sleep 6.5      # read the second reply

  # ── 7 · venues, help, out ──────────────────────────────────────────────
  k Tab; sleep 1           # leave the chat pane (typing captures ? and q there)
  k F9; sleep 2.2          # picker on the mode line: r rsx · h hyperliquid
  k h; sleep 4             # → live HL BTC book (real market, read-only)
  k F9; sleep 0.8; k r; sleep 1.5 # back to rsx
  k '?'; sleep 4.5         # keymap-generated help overlay
  k Escape; sleep 0.8
  k q; sleep 1
  tmux kill-session -t "$SESS" 2>/dev/null || true
} >/dev/null 2>&1 &
drv=$!
tmux attach -t "$SESS"
wait "$drv" 2>/dev/null || true
