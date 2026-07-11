#!/usr/bin/env bash
# Driven rsx-term walkthrough — recorded by asciinema (see demo/Makefile `gif`).
# Starts the terminal in a tmux pane on a live venue, walks its screens (BOOK
# heatmap → microscope freeze → assistant → NEWS → help), then quits so the
# recording ends cleanly. Default venue is phoenix.trade (real books, no local
# cluster needed); set RT_VENUE=rsx after `make demo` for the RSX 3-token book.
set -u
SESS=rtwalk
BIN="${RT_BIN:-/tmp/rt-walk}"
VENUE="${RT_VENUE:-phoenix}"

tmux kill-session -t "$SESS" 2>/dev/null || true
tmux new-session -d -s "$SESS" -x 118 -y 33
tmux set-option -t "$SESS" status off  # hide tmux chrome — record just the terminal
tmux send-keys -t "$SESS" "RSX_TERM_VENUE=$VENUE RSX_TERM_STREAM=1 RSX_TERM_NEWS=1 $BIN" Enter

{
  sleep 7                                            # boot: dial venue, books stream
  tmux send-keys -t "$SESS" Up Up Up Up Up; sleep 3  # BOOK microscope: scrub history
  tmux send-keys -t "$SESS" Enter;          sleep 5  # freeze row → assistant (LLM pane)
  tmux send-keys -t "$SESS" Tab;            sleep 5  # cycle screen
  tmux send-keys -t "$SESS" Tab;            sleep 5  # cycle screen (NEWS: majors + co-move)
  tmux send-keys -t "$SESS" "?";            sleep 5  # help overlay (keymap-generated)
  tmux send-keys -t "$SESS" Escape;         sleep 1  # close help
  tmux send-keys -t "$SESS" q;              sleep 1  # quit
  tmux kill-session -t "$SESS" 2>/dev/null || true
} &
drv=$!
tmux attach -t "$SESS"
wait "$drv" 2>/dev/null || true
