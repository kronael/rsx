.PHONY: rec gif clean

# Recording needs a QUIET box — a running RSX cluster (ME+Risk busy-spin)
# poisons Criterion's live measurements. Stop the cluster first, then `make rec`.
book-live.cast: bench-live.sh
	TERM=xterm-256color asciinema rec --overwrite --cols 46 --rows 24 \
		-c "bash bench-live.sh" book-live.cast

rec: book-live.cast

gif: book-live.cast
	agg --cols 46 --rows 24 --font-size 20 --idle-time-limit 1 \
		--theme monokai book-live.cast book-live.gif
	gifsicle -O3 book-live.gif -o book-live-opt.gif
	rm book-live.gif

clean:
	rm -f book-live.cast book-live.gif book-live-opt.gif
