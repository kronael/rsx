.PHONY: orderbook-gif dashboard-gif exchange-gif clean

# The terminal 3-token order-book board (run.py reads the authoritative
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

# The whole-exchange walkthrough: the ARCHITECTURE demo — an order traced
# across the processes, the two transports, the scale-out axes. Boots a
# fresh trio (several minutes) unless SKIP_PREP=1; renders exchange.gif.
exchange-gif:
	./exchange-walkthrough.sh

clean:
	rm -f orderbook.gif ../tmp/walkthrough/*.png ../tmp/*.cast
