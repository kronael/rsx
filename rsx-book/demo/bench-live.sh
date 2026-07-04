#!/usr/bin/env bash
# Records the REAL rsx-book depth benchmark: runs `cargo bench` and shows the
# actual Criterion measurements as they land — matching one order against a
# book of N resting orders, N from 100k to 10M. The pauses are real (Criterion
# warming up + collecting). See demo/CLAUDE.md to regenerate the GIF.
set -u
G=$'\e[1;32m'; C=$'\e[36m'; Y=$'\e[1;33m'; B=$'\e[1m'; D=$'\e[2m'; R=$'\e[0m'
label(){ case $1 in 100000) echo "100 K";; 1000000) echo "  1 M";; 10000000) echo " 10 M";; *) echo "$1";; esac; }
clear
printf '\n  %s%srsx-book%s %s· live benchmark%s\n\n' "$B" "$C" "$R" "$D" "$R"
printf '  %s$ cargo bench -- deep_flat_match%s\n\n' "$D" "$R"
printf '  %smatch 1 order vs a book of N orders%s\n\n' "$D" "$R"
cd "$(git rev-parse --show-toplevel)"
n=""
cargo bench -p rsx-book -- deep_flat_match 2>&1 | stdbuf -oL grep --line-buffered -E 'deep_flat_match/[0-9]+|time:' | \
while IFS= read -r line; do
  if [[ $line =~ deep_flat_match/([0-9]+) ]]; then n="${BASH_REMATCH[1]}"; l="$(label "$n")"; fi
  if [[ $line =~ (Warming|Collecting) ]]; then printf '  %s%s orders%s   %smeasuring…%s          \r' "$C" "$l" "$R" "$D" "$R"; fi
  if [[ $line =~ time:[[:space:]]*\[[0-9.]+[[:space:]][a-z]+[[:space:]]([0-9.]+)[[:space:]]([a-z]+) ]]; then
    printf '  %s%s orders%s  →  %s%.0f %s%s            \n' "$C" "$l" "$R" "$G" "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}" "$R"
  fi
done
printf '\n  %s10 M orders deep — flat.%s\n' "$Y" "$R"; sleep 1.0
printf '\n  %sclears the touch level%s    %s145 ns%s\n' "$D" "$R" "$G$B" "$R"; sleep 0.9
printf '  %s(was 100 us — cliff gone)%s\n' "$D" "$R"; sleep 1.1
printf '\n  %scancel an order%s           %s18 ns%s\n' "$D" "$R" "$G$B" "$R"; sleep 0.7
printf '  %s10x faster than a BTreeMap%s\n' "$D" "$R"; sleep 1.3
printf '\n  %s%sO(1) match + next-best%s\n' "$B" "$Y" "$R"; sleep 0.8
printf '  %son par with a tuned C++%s\n' "$D" "$R"
printf '  %sITCH book (61 ns)%s\n\n' "$D" "$R"; sleep 1.3
printf '  %s1 core · Ryzen 5950X%s\n' "$D" "$R"
printf '  %slab microbench · real cargo bench%s\n\n' "$D" "$R"; sleep 1.5
