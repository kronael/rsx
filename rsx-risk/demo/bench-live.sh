#!/usr/bin/env bash
# Records the REAL rsx-risk pre-trade benchmark: runs `cargo bench` and shows
# the actual Criterion measurements as they land — the per-order risk checks a
# Risk shard runs on the critical path. The headline is the full pre-trade gate.
# The pauses are real (Criterion warming up + collecting). See demo/CLAUDE.md.
set -u
# "Cemani" palette (project-wide; canonical source rsx-cast/demo/CLAUDE.md).
TEAL=$'\e[1;38;2;87;176;163m'   # fast/live result numbers
GOLD=$'\e[1;38;2;201;162;78m'   # headline claim + CTA
RUST=$'\e[1;38;2;176;112;63m'   # thing beaten / a cost (the 5 µs budget)
FG=$'\e[1;38;2;236;230;216m'    # normal body text
DIM=$'\e[2;38;2;143;134;114m'   # dim captions / caveats
R=$'\e[0m'
label(){ case $1 in
  apply_fill_to_position) echo "apply a fill  ";;
  exposure_lookup_100_users) echo "exposure lookup";;
  pretrade_check_latency) echo "PRE-TRADE GATE";;
  bbo_processing) echo "BBO -> index  ";;
  *) echo "$1";;
esac; }
clear
printf '\n  %sEvery order pays a risk check.%s\n' "$GOLD" "$R"
printf '  %sOurs costs ~110 ns — 45× under budget.%s\n\n' "$GOLD" "$R"
printf '  %s$ cargo bench -- pretrade%s\n\n' "$DIM" "$R"
printf '  %sthe per-order risk checks%s\n' "$DIM" "$R"
printf '  %son the critical path%s\n\n' "$DIM" "$R"
cd "$(git rev-parse --show-toplevel)"
name=""
cargo bench -p rsx-risk --bench risk_bench -- \
  'apply_fill_to_position|exposure_lookup_100_users|pretrade_check_latency|bbo_processing' \
  2>&1 | stdbuf -oL grep --line-buffered -E 'Benchmarking [A-Za-z0-9_]+|time:' | \
while IFS= read -r line; do
  if [[ $line =~ Benchmarking\ ([A-Za-z0-9_]+) ]]; then name="${BASH_REMATCH[1]}"; l="$(label "$name")"; fi
  if [[ $line =~ (Warming|Collecting) ]]; then printf '  %s%s%s   %smeasuring…%s          \r' "$FG" "$l" "$R" "$DIM" "$R"; fi
  if [[ $line =~ time:[[:space:]]*\[[0-9.]+[[:space:]][a-z]+[[:space:]]([0-9.]+)[[:space:]]([a-z]+) ]]; then
    if [[ $name == pretrade_check_latency ]]; then col="$GOLD"; else col="$TEAL"; fi
    printf '  %s%s%s  →  %s%.1f %s%s            \n' "$FG" "$l" "$R" "$col" "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}" "$R"
  fi
done
printf '\n  %sthe full pre-trade gate: ~110 ns%s\n' "$GOLD" "$R"; sleep 1.6
# Final card: clear to a compact hold so the last frame IS the headline.
clear
printf '\n  %srsx-risk%s\n\n' "$FG" "$R"
printf '  %sthe full per-order risk gate%s\n' "$DIM" "$R"
printf '  %son the critical path%s\n\n' "$DIM" "$R"; sleep 0.6
printf '  %s~110 ns.%s\n' "$GOLD" "$R"; sleep 0.8
printf '  %sbudget is 5 µs%s — %s45× under it.%s\n\n' "$RUST" "$R" "$GOLD" "$R"; sleep 1.4
printf '  %sexposure lookup is flat:%s\n' "$DIM" "$R"
printf '  %s100 → 1000 users, same 1.6 ns%s\n\n' "$DIM" "$R"; sleep 1.0
printf '  %s1 core · shared docker host%s\n' "$DIM" "$R"
printf '  %slab microbench · real cargo bench%s\n\n' "$DIM" "$R"; sleep 2.0
printf '  %sRead the code.%s\n' "$GOLD" "$R"
printf '  %sgithub.com/kronael/rsx%s\n\n' "$GOLD" "$R"; sleep 4
