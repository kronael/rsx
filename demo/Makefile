.PHONY: rec gif clean

COLS := 96
ROWS := 18
CAST := ../tmp/rsx-local-demo.cast
GIF := rsx-local-demo.gif

# The demo GIF is the 3-token live board (run.py) — the RSX system must already
# be up (`make demo` at the repo root) since run.py reads the live playground
# API for PENGU / SOL / BTC and paints their moving order books.
$(CAST): run.py
	mkdir -p ../tmp
	TERM=xterm-256color COLUMNS=$(COLS) LINES=$(ROWS) asciinema rec --overwrite \
		--cols $(COLS) --rows $(ROWS) \
		-c "../rsx-playground/.venv/bin/python3 run.py" $(CAST)

rec: $(CAST)

$(GIF): $(CAST)
	agg --theme github-dark --font-size 16 --cols $(COLS) --rows $(ROWS) \
		--idle-time-limit 2 --last-frame-duration 4 $(CAST) $(GIF)

gif: $(GIF)

clean:
	rm -f $(CAST) $(GIF)
