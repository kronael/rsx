.PHONY: gif orderbook-gif rec clean

# The demo GIF (README-embedded) is a comprehensive WALKTHROUGH of the live
# playground dashboard — browser screenshots of the key pages (Overview →
# Topology → Book → Orders → Maker → Latency → Cast → Terminal → Verify),
# assembled into a looping GIF. The RSX system must be up: `make demo`.
rsx-local-demo.gif: walkthrough.sh run.py
	./walkthrough.sh

gif: rsx-local-demo.gif

# Alternate: the terminal 3-token order-book board (run.py reads the authoritative
# marketdata WS) recorded via asciinema → agg.
COLS := 96
ROWS := 18
orderbook-gif: run.py
	mkdir -p ../tmp
	TERM=xterm-256color COLUMNS=$(COLS) LINES=$(ROWS) asciinema rec --overwrite \
		--cols $(COLS) --rows $(ROWS) \
		-c "../rsx-playground/.venv/bin/python3 run.py" ../tmp/orderbook.cast
	agg --theme github-dark --font-size 16 --cols $(COLS) --rows $(ROWS) \
		--idle-time-limit 2 --last-frame-duration 4 ../tmp/orderbook.cast orderbook.gif

clean:
	rm -f rsx-local-demo.gif orderbook.gif ../tmp/walkthrough/*.png ../tmp/*.cast
