#!/usr/bin/env bash
# Records the REAL rsx-matching engine live: the accept path (process_order —
# dedup + WAL append + match + event fan-out, per order) and cold-start
# recovery (wal_replay), Criterion medians as they land, then the live
# wire-to-wire GW->ME->GW round-trip taken from the embedded per-hop stamps
# (server-side; the Python client envelope is excluded). Numbers are never
# fabricated. See demo/CLAUDE.md to regenerate the GIF.
#
# WIRE_P50 comes from the server-side internal measurement in the gateway
# egress report. Record on a quiet box with the RSX cluster stopped.
set -euo pipefail
# "Cemani" palette (canonical source rsx-cast/demo/CLAUDE.md).
TEAL=$'\e[1;38;2;87;176;163m'   # live/cited result numbers
GOLD=$'\e[1;38;2;201;162;78m'   # headline claim + CTA
RUST=$'\e[1;38;2;176;112;63m'   # a cost / comparison-worst
FG=$'\e[1;38;2;236;230;216m'    # normal body text
DIM=$'\e[38;2;143;134;114m'     # muted captions / caveats
R=$'\e[0m'
cd "$(git rev-parse --show-toplevel)"

# median+unit from a Criterion `time:`/`thrpt:` line: the middle [lo med hi]
# pair. awk (not bash regex) so the µs unit's multibyte µ passes through clean.
crit(){ awk -v k="$1" '{for(i=1;i<=NF;i++) if($i==k){print $(i+3), $(i+4); exit}}'; }

# ── Cold-open: a neutron decays — it sheds a neutrino that flies off light
# and fast (TEAL, our ····▸ mascot), and the heavy p+e junk falls away
# (RUST). Simple parable: keep the fast light thing, drop the mass.
clear
printf '\e[?25l'  # hide the cursor for the whole reveal
printf '\n\n  %sa neutron decays:%s\n\n\n\n' "$FG" "$R"
printf '   %s( n )%s\n\n\n\n\n' "$FG" "$R"; sleep 1.5
clear
printf '\n\n  %sa neutron decays:%s\n\n' "$FG" "$R"
printf '            %s····▸%s\n' "$TEAL" "$R"
printf '           %s/%s\n' "$TEAL" "$R"
printf '   %s( n )%s\n' "$FG" "$R"
printf '           %s\\%s\n' "$RUST" "$R"
printf '            %sp e%s\n\n\n' "$RUST" "$R"
printf '  %sa neutrino flies off — light, fast,%s\n' "$DIM" "$R"
printf '  %sthrough everything. the junk falls away.%s\n\n' "$DIM" "$R"; sleep 2.8

clear
printf '\n  %srsx-matching%s\n' "$FG" "$R"
printf '  %sthe per-symbol matching engine%s\n\n' "$DIM" "$R"; sleep 0.6

# ── Beat 1: THE MATCH — the full accept path, per order ──────────────
printf '  %sTHE MATCH%s  %saccept -> match -> persist -> fan-out%s\n\n' "$GOLD" "$R" "$DIM" "$R"
printf '  %s$ cargo bench -- process_order%s\n\n' "$DIM" "$R"
stage=""
cargo bench -p rsx-matching --bench process_order_bench 2>&1 | \
  stdbuf -oL grep --line-buffered -E 'me_accept_path|me_throughput|time:|thrpt:' | \
while IFS= read -r line; do
  case $line in
    *me_accept_path*) stage=acc ;;
    *me_throughput*)  stage=thr ;;
  esac
  if [[ $line == *Warming* || $line == *Collecting* ]]; then
    printf '  %smeasuring…%s                    \r' "$DIM" "$R"
  fi
  if [[ $line == *time:* && $stage == acc ]]; then
    read -r v u < <(printf '%s\n' "$line" | crit time:)
    printf '  %sper order%s   ->  %s%s %s%s          \n' "$FG" "$R" "$TEAL" "$v" "$u" "$R"
  fi
  if [[ $line == *thrpt:* && $line == *lem/s* && $stage == thr && -z ${thr_seen:-} ]]; then
    thr_seen=1
    read -r v u < <(printf '%s\n' "$line" | crit thrpt:)
    printf '  %sthroughput%s  ->  %s%s %s%s          \n' "$FG" "$R" "$TEAL" "$v" "$u" "$R"
  fi
done
printf '\n  %sdedup + WAL + match + fan-out, per order%s\n\n' "$DIM" "$R"; sleep 1.4

# ── Beat 2: RECOVERY — cold-start WAL replay before order #1 ─────────
clear
printf '\n  %sRECOVERY%s  %scold-start: rebuild before order #1%s\n\n' "$GOLD" "$R" "$DIM" "$R"
printf '  %s$ cargo bench -- wal_replay%s\n\n' "$DIM" "$R"
cargo bench -p rsx-matching --bench wal_replay_bench 2>&1 | \
  stdbuf -oL grep --line-buffered -E 'wal_replay|time:' | \
while IFS= read -r line; do
  if [[ $line == *Warming* || $line == *Collecting* ]]; then
    printf '  %s30 K records%s   %sreplaying…%s          \r' "$FG" "$R" "$DIM" "$R"
  fi
  if [[ $line == *time:* ]]; then
    read -r v u < <(printf '%s\n' "$line" | crit time:)
    printf '  %s30 K records%s  ->  %s%s %s%s          \n' "$FG" "$R" "$TEAL" "$v" "$u" "$R"
  fi
done
printf '\n  %sreplay order = disk order = match order (FIFO)%s\n\n' "$DIM" "$R"; sleep 1.4

# ── Beat 3: WIRE-TO-WIRE — live GW->ME->GW, embedded per-hop stamps ──
# WIRE_P50: live-cluster internal round-trip (gw_out-gw_in) p50 from the
# embedded stamps, release binaries (reports/20260723_gateway-egress.md,
# clean-window sample). The matching leg is 12us of it; the rest is
# transport. NOT sub-50us — that was bench-match-rt's 2-hop model (no risk
# tile). The honest full-path number is sub-millisecond.
WIRE_P50="~0.65 ms"
clear
printf '\n  %sWIRE-TO-WIRE%s  %sGW -> risk -> ME -> risk -> GW%s\n\n' "$GOLD" "$R" "$DIM" "$R"
printf '  %slive cluster · per-hop stamps in the order%s\n' "$DIM" "$R"
printf '  %sserver-side; Python client excluded%s\n\n' "$DIM" "$R"
printf '  %sround-trip p50%s  ->  %s%s%s\n' "$FG" "$R" "$TEAL" "$WIRE_P50" "$R"
printf '  %smatching leg: 12 us · the rest is transport%s\n' "$DIM" "$R"
printf '  %ssingle clean window · shared host%s\n\n' "$DIM" "$R"; sleep 1.8

# ── Close ───────────────────────────────────────────────────────────
clear
printf '\n  %srsx-matching%s\n\n' "$FG" "$R"
printf '  %smatch · recover · round-trip%s\n\n' "$DIM" "$R"; sleep 0.6
printf '  %s····▸%s %srsx%s\n\n' "$TEAL" "$R" "$GOLD" "$R"
printf '  %ssingle shard · lab microbench + live stamps%s\n\n' "$DIM" "$R"; sleep 1.6
printf '\e[?25l\n\n\n  %sRead the code.%s\n' "$GOLD" "$R"
printf '  %sgithub.com/kronael/rsx%s\n\n' "$GOLD" "$R"; sleep 4
