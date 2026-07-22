#!/usr/bin/env bash
# Records the REAL rsx-book depth benchmark: runs `cargo bench` and shows the
# actual Criterion measurements as they land — matching one order against a
# book of N resting orders, N from 100k to 10M. The pauses are real (Criterion
# warming up + collecting). See demo/CLAUDE.md to regenerate the GIF.
set -euo pipefail
# "Cemani" palette (project-wide; canonical source rsx-cast/demo/CLAUDE.md).
TEAL=$'\e[1;38;2;87;176;163m'   # fast/live result numbers
GOLD=$'\e[1;38;2;201;162;78m'   # headline claim + CTA
RUST=$'\e[1;38;2;176;112;63m'   # thing beaten / cited C++
FG=$'\e[1;38;2;236;230;216m'    # normal body text
DIM=$'\e[38;2;143;134;114m'     # muted captions / caveats
R=$'\e[0m'
label(){ case $1 in 100000) echo "100 K";; 1000000) echo "  1 M";; 10000000) echo " 10 M";; *) echo "$1";; esac; }

# ── cold-open: the expanding-brain of order-book design (naive → galaxy).
# The ░▒▓█ ramp fills as the design gets smarter; palette runs worst→best
# (rust=beaten, teal=the winner rsx-book ships). One clear → the live bench.
clear
printf '\n  %show do you make an order book fast?%s\n\n' "$GOLD" "$R"; sleep 1.2
printf '  %s░%s    %sBTreeMap<px, deque>%s\n'            "$RUST" "$R" "$RUST" "$R"; sleep 0.7
printf '        %stree descent, every op%s\n\n'          "$DIM"  "$R"; sleep 0.8
printf '  %s▒%s    %sflat price array%s\n'               "$FG"   "$R" "$FG"   "$R"; sleep 0.7
printf '        %sO(1) index, O(range) to find%s\n\n'    "$DIM"  "$R"; sleep 0.8
printf '  %s▓%s    %sslab arena%s\n'                     "$FG"   "$R" "$FG"   "$R"; sleep 0.7
printf '        %salloc = bump / free-list pop%s\n\n'    "$DIM"  "$R"; sleep 0.8
printf '  %s█%s    %sslab + distance compression%s\n'    "$TEAL" "$R" "$TEAL" "$R"; sleep 0.7
printf '        %sO(1) everything — depth-flat%s\n\n'    "$DIM"  "$R"; sleep 1.3
printf '  %sso — how fast, 10 M orders deep?%s\n'         "$GOLD" "$R"; sleep 1.7

clear
printf '\n\n  %sOne match stays in the same band —%s\n' "$GOLD" "$R"
printf '  %sfrom 100 K to 10 M resting orders.%s\n\n' "$GOLD" "$R"
printf '  %s$ cargo bench -- deep_flat_match%s\n\n' "$DIM" "$R"
cd "$(git rev-parse --show-toplevel)"
results="$(mktemp)"
trap 'rm -f "$results"' EXIT
n=""
cargo bench -p rsx-book --bench deep_book_bench -- deep_flat_match 2>&1 | stdbuf -oL grep --line-buffered -E 'deep_flat_match/[0-9]+|time:' | \
while IFS= read -r line; do
  if [[ $line =~ deep_flat_match/([0-9]+) ]]; then n="${BASH_REMATCH[1]}"; l="$(label "$n")"; fi
  if [[ $line =~ (Warming|Collecting) ]]; then printf '  %s%s orders%s   %smeasuring…%s          \r' "$FG" "$l" "$R" "$DIM" "$R"; fi
  if [[ $line =~ time:[[:space:]]*\[[0-9.]+[[:space:]][a-z]+[[:space:]]([0-9.]+)[[:space:]]([a-z]+) ]]; then
    printf '%s %s\n' "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}" >> "$results"
    printf '  %s%s orders%s  →  %s%.0f %s%s            \n' "$FG" "$l" "$R" "$TEAL" "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}" "$R"
  fi
done
[[ $(wc -l < "$results") -eq 3 ]]
range="$(awk '$2 == "ns" { if (!n || $1 < lo) lo=$1; if (!n || $1 > hi) hi=$1; n++ } END { if (n != 3) exit 1; printf "%.0f–%.0f ns", lo, hi }' "$results")"
printf '\n  %s100 K → 10 M: %s · same band%s\n' "$TEAL" "$range" "$R"; sleep 1.2
printf '\n  %sO(1) lookup — depth-invariant by design%s\n\n' "$DIM" "$R"; sleep 1.1
printf '  %s1 core · shared Ryzen 5950X host%s\n' "$DIM" "$R"
printf '  %slab microbench · real cargo bench%s\n\n' "$DIM" "$R"; sleep 1.5
# rsx signature — the neutrino train: an invisible near-lightspeed particle, so
# only its wake shows — a trail of dots streaking into the wordmark.
printf '\e[?25l  %s····▸%s %srsx%s\n\n' "$TEAL" "$R" "$GOLD" "$R"; sleep 0.9
printf '  %sRead the code.%s\n' "$GOLD" "$R"
printf '  %sgithub.com/kronael/rsx%s\n\n' "$GOLD" "$R"; sleep 4
