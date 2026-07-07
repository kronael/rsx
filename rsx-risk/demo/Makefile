.PHONY: rec gif clean

# Recording needs a QUIET box — a running RSX cluster (ME+Risk busy-spin)
# poisons Criterion's live measurements. Stop the cluster first, then `make rec`.
risk-live.cast: bench-live.sh
	TERM=xterm-256color asciinema rec --overwrite --cols 46 --rows 24 \
		-c "bash bench-live.sh" risk-live.cast

rec: risk-live.cast

gif: risk-live.cast
	agg --cols 46 --rows 24 --font-size 20 --idle-time-limit 1 \
		--theme monokai risk-live.cast risk-live.gif
	gifsicle -O3 risk-live.gif -o risk-live-opt.gif
	rm risk-live.gif

clean:
	rm -f risk-live.cast risk-live.gif risk-live-opt.gif
