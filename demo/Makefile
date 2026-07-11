.PHONY: gif orderbook-gif dashboard-gif clean

# The README-embedded demo GIF is a driven WALKTHROUGH of the rsx-term trading
# terminal (BOOK heatmap → microscope scrub → freeze → assistant → NEWS → help)
# on a live venue. Default is phoenix.trade — real books, no local cluster
# needed; set RT_VENUE=rsx (after `make demo` brings the 3-token cluster up) for
# the RSX book. asciinema records terminal.sh; agg renders. See the `demo` skill.
COLS := 118
ROWS := 33
RT_BIN := /tmp/rt-walk
rsx-local-demo.gif: terminal.sh
	cd .. && go build -o $(RT_BIN) ./rsx-term
	mkdir -p ../tmp
	TERM=xterm-256color RT_BIN=$(RT_BIN) asciinema rec --overwrite \
		--cols $(COLS) --rows $(ROWS) -c "bash terminal.sh" ../tmp/terminal.cast
	agg --theme github-dark --font-size 14 --cols $(COLS) --rows $(ROWS) \
		--idle-time-limit 2 --last-frame-duration 3 ../tmp/terminal.cast rsx-local-demo.gif

gif: rsx-local-demo.gif

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
	rm -f rsx-local-demo.gif orderbook.gif ../tmp/walkthrough/*.png ../tmp/*.cast
