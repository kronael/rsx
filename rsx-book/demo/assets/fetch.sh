#!/usr/bin/env bash
# One-shot: fetch 4 replacement character images -> render -> build the clip.
#
#   bash fetch.sh <neanderthal-url> <engineer-url> <genius-url> <gigachad-url>
#   bash fetch.sh                 # reads urls.txt (one "slot url" pair per line)
#
set -euo pipefail
cd "$(dirname "$0")"

SLOTS=(neanderthal engineer genius gigachad)
UA='Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126 Safari/537.36'

declare -A URL
if [ "$#" -eq 4 ]; then
	for i in 0 1 2 3; do URL[${SLOTS[$i]}]="${@:i+1:1}"; done
elif [ "$#" -eq 0 ] && [ -f urls.txt ]; then
	while read -r slot url; do
		case "$slot" in ''|'#'*) continue ;; esac
		URL[$slot]="$url"
	done < urls.txt
else
	echo "usage: bash fetch.sh <4 urls>   |   put them in urls.txt and run bare" >&2
	echo "slots: ${SLOTS[*]}" >&2
	exit 2
fi

for slot in "${SLOTS[@]}"; do
	url="${URL[$slot]:-}"
	[ -n "$url" ] || { echo "no url for slot '$slot'" >&2; exit 1; }

	tmp="$(mktemp)"
	curl -fsSL --max-time 60 -A "$UA" -o "$tmp" "$url" \
		|| { echo "download failed: $slot <- $url" >&2; exit 1; }

	case "$(file --mime-type -b "$tmp")" in
		image/png) ext=png ;;
		image/jpeg) ext=jpg ;;
		image/webp) ext=webp ;;
		*) echo "$slot: not an image ($(file -b "$tmp")) — that URL is a web page, not the file. Right-click > Copy image address." >&2; exit 1 ;;
	esac

	# the loader globs png|jpg|jpeg|webp — drop stale slots so the new one wins
	rm -f "$slot.png" "$slot.jpg" "$slot.jpeg" "$slot.webp"
	mv "$tmp" "$slot.$ext"
	echo "  $slot.$ext  ($(wc -c < "$slot.$ext") bytes)"
done

uv run gen_collage.py
bash build.sh
echo
echo "next: point rsx-book at rsx-book/demo/assets/book-versus.gif in"
echo "rsx-playground/pages.py, then restart the dashboard."
