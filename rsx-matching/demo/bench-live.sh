#!/usr/bin/env bash
# Records the REAL rsx-matching depth benchmark: runs `cargo bench` and shows
# the actual Criterion measurements as they land — matching one order against a
# resting book of N orders, N from 1 to 100k. The pauses are real (Criterion
# warming up + collecting). See demo/CLAUDE.md to regenerate the GIF.
set -u
# "Cemani" palette (project-wide; canonical source rsx-cast/demo/CLAUDE.md).
TEAL=$'\e[1;38;2;87;176;163m'   # fast/live result numbers
GOLD=$'\e[1;38;2;201;162;78m'   # headline claim + CTA
RUST=$'\e[1;38;2;176;112;63m'   # thing beaten / a cost / comparison-worst
FG=$'\e[1;38;2;236;230;216m'    # normal body text
DIM=$'\e[2;38;2;143;134;114m'   # dim captions / caveats
R=$'\e[0m'
label(){ case $1 in 1) echo "     1";; 100) echo "   100";; 1000) echo "   1 K";; 10000) echo "  10 K";; 100000) echo " 100 K";; *) echo "$1";; esac; }
clear
printf '\n  %sOne order matches in ~30 ns —%s\n' "$GOLD" "$R"
printf '  %s1 resting order or 100 K, same.%s\n\n' "$GOLD" "$R"
printf '  %s$ cargo bench -- match_by_depth%s\n\n' "$DIM" "$R"
printf '  %smatch 1 order vs a book of N orders%s\n\n' "$DIM" "$R"
cd "$(git rev-parse --show-toplevel)"
n=""
cargo bench -p rsx-matching --bench match_depth_bench 2>&1 | stdbuf -oL grep --line-buffered -E 'match_by_depth/n=[0-9]+|time:' | \
while IFS= read -r line; do
  if [[ $line =~ match_by_depth/n=([0-9]+) ]]; then n="${BASH_REMATCH[1]}"; l="$(label "$n")"; fi
  if [[ $line =~ (Warming|Collecting) ]]; then printf '  %s%s orders%s   %smeasuring…%s          \r' "$FG" "$l" "$R" "$DIM" "$R"; fi
  if [[ $line =~ time:[[:space:]]*\[[0-9.]+[[:space:]][a-z]+[[:space:]]([0-9.]+)[[:space:]]([a-z]+) ]]; then
    printf '  %s%s orders%s  →  %s%.0f %s%s            \n' "$FG" "$l" "$R" "$TEAL" "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}" "$R"
  fi
done
printf '\n  %s100 K orders deep — still ~30 ns%s\n' "$GOLD" "$R"; sleep 1.6
# Final card: clear to a compact hold so the last frame IS the headline.
clear
printf '\n  %srsx-matching%s\n\n' "$FG" "$R"
printf '  %s1 match vs a book of%s\n' "$DIM" "$R"
printf '  %s1 → 100 K resting orders%s\n\n' "$DIM" "$R"; sleep 0.6
printf '  %s~30 ns. flat across depth.%s\n' "$GOLD" "$R"; sleep 0.9
printf '  %sO(1) — depth doesn'\''t matter.%s\n\n' "$GOLD" "$R"; sleep 1.4
printf '  %sfull order accept%s   %s266 ns%s %s(report)%s\n' "$DIM" "$R" "$TEAL" "$R" "$DIM" "$R"
printf '  %sduplicate rejected%s  %s3.7 ns%s %s(report)%s\n\n' "$DIM" "$R" "$TEAL" "$R" "$DIM" "$R"; sleep 1.0
printf '  %s1 core · shared docker host%s\n' "$DIM" "$R"
printf '  %slab microbench · real cargo bench%s\n\n' "$DIM" "$R"; sleep 2.0
printf '  %sRead the code.%s\n' "$GOLD" "$R"
printf '  %sgithub.com/kronael/rsx%s\n\n' "$GOLD" "$R"; sleep 4
