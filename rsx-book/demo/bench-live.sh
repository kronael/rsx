#!/usr/bin/env bash
# Records the REAL rsx-book depth benchmark: runs `cargo bench` and shows the
# actual Criterion measurements as they land — matching one order against a
# book of N resting orders, N from 100k to 10M. The pauses are real (Criterion
# warming up + collecting). See demo/CLAUDE.md to regenerate the GIF.
set -u
# "Cemani" palette (project-wide; canonical source rsx-cast/demo/CLAUDE.md).
TEAL=$'\e[1;38;2;87;176;163m'   # fast/live result numbers
GOLD=$'\e[1;38;2;201;162;78m'   # headline claim + CTA
RUST=$'\e[1;38;2;176;112;63m'   # thing beaten / cited C++
FG=$'\e[1;38;2;236;230;216m'    # normal body text
DIM=$'\e[2;38;2;143;134;114m'   # dim captions / caveats
R=$'\e[0m'
label(){ case $1 in 100000) echo "100 K";; 1000000) echo "  1 M";; 10000000) echo " 10 M";; *) echo "$1";; esac; }
clear
printf '\n  %sOne order matches in ~60 ns —%s\n' "$GOLD" "$R"
printf '  %swhether the book holds 100 K or 10 M.%s\n\n' "$GOLD" "$R"
printf '  %s$ cargo bench -- deep_flat_match%s\n\n' "$DIM" "$R"
printf '  %smatch 1 order vs a book of N orders%s\n\n' "$DIM" "$R"
cd "$(git rev-parse --show-toplevel)"
n=""
cargo bench -p rsx-book -- deep_flat_match 2>&1 | stdbuf -oL grep --line-buffered -E 'deep_flat_match/[0-9]+|time:' | \
while IFS= read -r line; do
  if [[ $line =~ deep_flat_match/([0-9]+) ]]; then n="${BASH_REMATCH[1]}"; l="$(label "$n")"; fi
  if [[ $line =~ (Warming|Collecting) ]]; then printf '  %s%s orders%s   %smeasuring…%s          \r' "$FG" "$l" "$R" "$DIM" "$R"; fi
  if [[ $line =~ time:[[:space:]]*\[[0-9.]+[[:space:]][a-z]+[[:space:]]([0-9.]+)[[:space:]]([a-z]+) ]]; then
    printf '  %s%s orders%s  →  %s%.0f %s%s            \n' "$FG" "$l" "$R" "$TEAL" "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}" "$R"
  fi
done
printf '\n  %s10 M orders — still ~60 ns · flat%s\n' "$GOLD" "$R"; sleep 1.2
printf '\n  %sfrom the report:%s\n' "$DIM" "$R"; sleep 0.5
printf '  %sclears the touch level%s    %s145 ns%s\n' "$DIM" "$R" "$TEAL" "$R"; sleep 0.9
printf '  %snext-best is O(depth), flat%s\n' "$DIM" "$R"; sleep 1.1
printf '  %scancel an order%s           %s18 ns%s\n' "$DIM" "$R" "$TEAL" "$R"; sleep 0.7
printf '  %sup to 10x vs a BTreeMap%s\n' "$RUST" "$R"; sleep 1.3
printf '\n  %scited only · C++ ITCH book%s\n' "$RUST" "$R"; sleep 0.6
printf '  %s61 ns/tick — 2012 HW,%s\n' "$DIM" "$R"
printf '  %sbook-maintenance, not matching%s\n' "$DIM" "$R"
printf '  %s(fair line: our insert+cancel)%s\n\n' "$DIM" "$R"; sleep 1.3
printf '  %s1 core · Ryzen 5950X%s\n' "$DIM" "$R"
printf '  %slab microbench · real cargo bench%s\n\n' "$DIM" "$R"; sleep 1.5
printf '  %sRead the code.%s\n' "$GOLD" "$R"
printf '  %sgithub.com/kronael/rsx%s\n\n' "$GOLD" "$R"; sleep 4
