.PHONY: rec gif clean

# Comprehensive driven walkthrough of the rsx-term terminal on the LIVE RSX
# cluster (books, trading keys, microscope→assistant, news, venues — see
# walk.sh's header). walk.sh drives it in a headless tmux pane; asciinema
# records, agg renders, gifsicle optimizes. Needs Go (builds the binary),
# tmux, and the cluster up with live books:
#     ./rsx-playground/playground reset --yes && make demo   (repo root)
# For the REAL assistant round-trip export RSX_TERM_ASSIST (a live arizuko
# /chat/{token} URL); `rec` best-effort mints one via chat-token.sh when
# unset. Keep COLS/ROWS in sync with walk.sh's tmux size or the GIF crops.
RT_BIN := /tmp/rt-walk
COLS := 118
ROWS := 33
TOKEN_SH := ../../.ship/45-ARIZUKO-LLM/chat-token.sh

term-live.cast: walk.sh
	cd .. && go build -o $(RT_BIN) .
	@curl -sf -m 2 -o /dev/null http://127.0.0.1:8088/health \
		|| { echo "RSX gateway :8088 not up — run 'make demo' from the repo root first" >&2; exit 1; }
	@assist="$${RSX_TERM_ASSIST:-$$($(TOKEN_SH) 2>/dev/null | sed -n 's/^RSX_TERM_ASSIST=//p')}"; \
	 [ -n "$$assist" ] || echo "warning: no RSX_TERM_ASSIST — assistant pane records the offline placeholder" >&2; \
	 TERM=xterm-256color RT_BIN=$(RT_BIN) RSX_TERM_ASSIST="$$assist" \
		asciinema rec --overwrite --cols $(COLS) --rows $(ROWS) \
		-c "bash walk.sh" term-live.cast

rec: term-live.cast

gif: term-live.cast
	agg --theme github-dark --font-size 14 --cols $(COLS) --rows $(ROWS) \
		--idle-time-limit 2 --last-frame-duration 3 term-live.cast term-live.gif
	gifsicle -O3 term-live.gif -o term-live-opt.gif

clean:
	rm -f term-live.cast term-live.gif term-live-opt.gif
