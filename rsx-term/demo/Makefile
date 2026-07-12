.PHONY: rec gif clean

# Driven walkthrough of the rsx-term terminal on a live venue (phoenix.trade by
# default — real books, no local cluster). walk.sh drives it in a headless tmux
# pane; asciinema records, agg renders, gifsicle optimizes. Needs Go (builds the
# binary), tmux, and a network path to the venue. Set RT_VENUE=rsx (after
# `make demo` from the repo root) to record against the RSX 3-token book.
# Keep COLS/ROWS in sync with walk.sh's tmux size or the GIF crops.
RT_BIN := /tmp/rt-walk
COLS := 118
ROWS := 33

term-live.cast: walk.sh
	cd ../.. && go build -o $(RT_BIN) ./rsx-term
	TERM=xterm-256color RT_BIN=$(RT_BIN) asciinema rec --overwrite \
		--cols $(COLS) --rows $(ROWS) -c "bash walk.sh" term-live.cast

rec: term-live.cast

gif: term-live.cast
	agg --theme github-dark --font-size 14 --cols $(COLS) --rows $(ROWS) \
		--idle-time-limit 2 --last-frame-duration 3 term-live.cast term-live.gif
	gifsicle -O3 term-live.gif -o term-live-opt.gif

clean:
	rm -f term-live.cast term-live.gif term-live-opt.gif
