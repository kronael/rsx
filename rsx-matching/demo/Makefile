.PHONY: rec gif clean

# Recording needs a QUIET box — a running RSX cluster (ME+Risk busy-spin)
# poisons Criterion's live measurements. Stop the cluster first, then `make rec`.
match-live.cast: bench-live.sh
	TERM=xterm-256color asciinema rec --overwrite --cols 46 --rows 24 \
		-c "bash bench-live.sh" match-live.cast

rec: match-live.cast

gif: match-live.cast
	agg --cols 46 --rows 24 --font-size 20 --idle-time-limit 1 \
		--theme monokai match-live.cast match-live.gif
	gifsicle -O3 match-live.gif -o match-live-opt.gif

clean:
	rm -f match-live.cast match-live.gif match-live-opt.gif
