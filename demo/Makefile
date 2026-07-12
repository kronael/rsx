.PHONY: gif orderbook-gif dashboard-gif clean

# The README-embedded terminal walkthrough now lives with its crate:
# rsx-term/demo/ (walk.sh → term-live.gif/-opt). This is a convenience alias
# that regenerates it there; the root README embeds rsx-term/demo/term-live-opt.gif.
gif:
	$(MAKE) -C ../rsx-term/demo gif

# Alternate: the terminal 3-token order-book board (run.py reads the authoritative
# marketdata WS) recorded via asciinema → agg.
OB_COLS := 96
OB_ROWS := 18
orderbook-gif: run.py
	mkdir -p ../tmp
	TERM=xterm-256color COLUMNS=$(OB_COLS) LINES=$(OB_ROWS) asciinema rec --overwrite \
		--cols $(OB_COLS) --rows $(OB_ROWS) \
		-c "../rsx-playground/.venv/bin/python3 run.py" ../tmp/orderbook.cast
	agg --theme github-dark --font-size 16 --cols $(OB_COLS) --rows $(OB_ROWS) \
		--idle-time-limit 2 --last-frame-duration 4 ../tmp/orderbook.cast orderbook.gif

# The dashboard walkthrough now lives in the playground (browser screenshots of
# the live dashboard pages). Kept here as a convenience alias.
dashboard-gif:
	../rsx-playground/demo/dashboard-walkthrough.sh

clean:
	rm -f orderbook.gif ../tmp/walkthrough/*.png ../tmp/*.cast
