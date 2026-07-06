.PHONY: rec gif clean

cast-live.cast: pitch.py
	TERM=xterm-256color asciinema rec --overwrite --cols 44 --rows 24 \
		-c "uv run pitch.py" cast-live.cast

rec: cast-live.cast

gif: cast-live.cast
	agg --cols 44 --rows 24 --font-size 20 --idle-time-limit 1 \
		--theme monokai cast-live.cast cast-live.gif
	gifsicle -O3 cast-live.gif -o cast-live-opt.gif

clean:
	rm -f cast-live.cast cast-live.gif cast-live-opt.gif
