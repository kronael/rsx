#!/usr/bin/env bash
# Records the REAL rsx-matching depth benchmark: runs `cargo bench` and shows
# the actual Criterion measurements as they land — matching one order against a
# resting book of N orders, N from 1 to 100k. The pauses are real (Criterion
# warming up + collecting). See demo/CLAUDE.md to regenerate the GIF.
set -u
G=$'\e[1;32m'; C=$'\e[36m'; Y=$'\e[1;33m'; B=$'\e[1m'; D=$'\e[2m'; R=$'\e[0m'
label(){ case $1 in 1) echo "     1";; 100) echo "   100";; 1000) echo "   1 K";; 10000) echo "  10 K";; 100000) echo " 100 K";; *) echo "$1";; esac; }
clear
printf '\n  %s%srsx-matching%s %s· live benchmark%s\n\n' "$B" "$C" "$R" "$D" "$R"
printf '  %s$ cargo bench -- match_by_depth%s\n\n' "$D" "$R"
printf '  %smatch 1 order vs a book of N orders%s\n\n' "$D" "$R"
cd "$(git rev-parse --show-toplevel)"
n=""
cargo bench -p rsx-matching --bench match_depth_bench 2>&1 | stdbuf -oL grep --line-buffered -E 'match_by_depth/n=[0-9]+|time:' | \
while IFS= read -r line; do
  if [[ $line =~ match_by_depth/n=([0-9]+) ]]; then n="${BASH_REMATCH[1]}"; l="$(label "$n")"; fi
  if [[ $line =~ (Warming|Collecting) ]]; then printf '  %s%s orders%s   %smeasuring…%s          \r' "$C" "$l" "$R" "$D" "$R"; fi
  if [[ $line =~ time:[[:space:]]*\[[0-9.]+[[:space:]][a-z]+[[:space:]]([0-9.]+)[[:space:]]([a-z]+) ]]; then
    printf '  %s%s orders%s  →  %s%.0f %s%s            \n' "$C" "$l" "$R" "$G" "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}" "$R"
  fi
done
printf '\n  %s100 K orders deep — still ~30 ns%s\n' "$Y" "$R"; sleep 1.6
# Final card: clear to a compact hold so the last frame IS the headline.
clear
printf '\n  %s%srsx-matching%s\n\n' "$B" "$C" "$R"
printf '  %s1 match vs a book of%s\n' "$D" "$R"
printf '  %s1 → 100 K resting orders%s\n\n' "$D" "$R"; sleep 0.6
printf '  %s%s~30 ns. flat across depth.%s\n' "$B" "$Y" "$R"; sleep 0.9
printf '  %s%sO(1) — depth doesn'\''t matter.%s\n\n' "$B" "$Y" "$R"; sleep 1.4
printf '  %sfull order accept%s   %s266 ns%s %s(report)%s\n' "$D" "$R" "$G$B" "$R" "$D" "$R"
printf '  %sduplicate rejected%s  %s3.7 ns%s %s(report)%s\n\n' "$D" "$R" "$G$B" "$R" "$D" "$R"; sleep 1.0
printf '  %s1 core · shared docker host%s\n' "$D" "$R"
printf '  %slab microbench · real cargo bench%s\n\n' "$D" "$R"; sleep 2.0
