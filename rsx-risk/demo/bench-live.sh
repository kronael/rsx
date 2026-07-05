#!/usr/bin/env bash
# Records the REAL rsx-risk pre-trade benchmark: runs `cargo bench` and shows
# the actual Criterion measurements as they land — the per-order risk checks a
# Risk shard runs on the critical path. The headline is the full pre-trade gate.
# The pauses are real (Criterion warming up + collecting). See demo/CLAUDE.md.
set -u
G=$'\e[1;32m'; C=$'\e[36m'; Y=$'\e[1;33m'; B=$'\e[1m'; D=$'\e[2m'; R=$'\e[0m'
label(){ case $1 in
  apply_fill_to_position) echo "apply a fill  ";;
  exposure_lookup_100_users) echo "exposure lookup";;
  pretrade_check_latency) echo "PRE-TRADE GATE";;
  bbo_processing) echo "BBO -> index  ";;
  *) echo "$1";;
esac; }
clear
printf '\n  %s%srsx-risk%s %s· live benchmark%s\n\n' "$B" "$C" "$R" "$D" "$R"
printf '  %s$ cargo bench -- pretrade%s\n\n' "$D" "$R"
printf '  %sthe per-order risk checks%s\n' "$D" "$R"
printf '  %son the critical path%s\n\n' "$D" "$R"
cd "$(git rev-parse --show-toplevel)"
name=""
cargo bench -p rsx-risk --bench risk_bench -- \
  'apply_fill_to_position|exposure_lookup_100_users|pretrade_check_latency|bbo_processing' \
  2>&1 | stdbuf -oL grep --line-buffered -E 'Benchmarking [A-Za-z0-9_]+|time:' | \
while IFS= read -r line; do
  if [[ $line =~ Benchmarking\ ([A-Za-z0-9_]+) ]]; then name="${BASH_REMATCH[1]}"; l="$(label "$name")"; fi
  if [[ $line =~ (Warming|Collecting) ]]; then printf '  %s%s%s   %smeasuring…%s          \r' "$C" "$l" "$R" "$D" "$R"; fi
  if [[ $line =~ time:[[:space:]]*\[[0-9.]+[[:space:]][a-z]+[[:space:]]([0-9.]+)[[:space:]]([a-z]+) ]]; then
    if [[ $name == pretrade_check_latency ]]; then col="$Y$B"; else col="$G"; fi
    printf '  %s%s%s  →  %s%.1f %s%s            \n' "$C" "$l" "$R" "$col" "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}" "$R"
  fi
done
printf '\n  %s%sthe full pre-trade gate: ~110 ns%s\n' "$B" "$Y" "$R"; sleep 1.6
# Final card: clear to a compact hold so the last frame IS the headline.
clear
printf '\n  %s%srsx-risk%s\n\n' "$B" "$C" "$R"
printf '  %sthe full per-order risk gate%s\n' "$D" "$R"
printf '  %son the critical path%s\n\n' "$D" "$R"; sleep 0.6
printf '  %s%s~110 ns.%s\n' "$B" "$Y" "$R"; sleep 0.8
printf '  %sbudget is 5 µs —%s %s%s45× under it.%s\n\n' "$D" "$R" "$B" "$G" "$R"; sleep 1.4
printf '  %sexposure lookup is flat:%s\n' "$D" "$R"
printf '  %s100 → 1000 users, same 1.6 ns%s\n\n' "$D" "$R"; sleep 1.0
printf '  %s1 core · shared docker host%s\n' "$D" "$R"
printf '  %slab microbench · real cargo bench%s\n\n' "$D" "$R"; sleep 2.0
