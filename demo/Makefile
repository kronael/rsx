.PHONY: rec gif clean

COLS := 88
ROWS := 26
CAST := ../tmp/rsx-local-demo.cast
GIF := rsx-local-demo.gif

$(CAST): run.sh
	mkdir -p ../tmp
	TERM=xterm-256color asciinema rec --overwrite \
		--cols $(COLS) --rows $(ROWS) \
		-c "bash run.sh" $(CAST)

rec: $(CAST)

$(GIF): $(CAST) render_cast.py
	python3 render_cast.py $(CAST) $(GIF) --cols $(COLS) --rows $(ROWS)

gif: $(GIF)

clean:
	rm -f $(CAST) $(GIF)
