#!/usr/bin/env bash
# play-shards-report.sh - Run all domain shards; publish combined report.
#
# Runs each shard via play-shard.sh (continues on failure).
# After all shards complete, aggregates per-shard summary.txt files
# into a single consolidated report with pass/fail counts and failing IDs.
#
# Artifacts:
#   tmp/play-artifacts/<shard>/summary.txt   per-shard (from play-shard.sh)
#   tmp/play-artifacts/shards-report/report.txt  combined report
#
# Exit codes:
#   0  all shards passed
#   1  one or more shards failed (new or changed failures)
#   2  one or more shards blocked (same sig, no domain change)

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SHARD_SCRIPT="$SCRIPT_DIR/play-shard.sh"
REPORT_DIR="$SCRIPT_DIR/../tmp/play-artifacts/shards-report"
mkdir -p "$REPORT_DIR"
REPORT_FILE="$REPORT_DIR/report.txt"

# Ordered list of shards — infra-smoke gates; product shards follow.
SHARDS=(
    infra-smoke
    routing
    htmx-partials
    process-control
    market-maker
    trade-ui
)

TS="$(date '+%b %d %H:%M:%S')"
echo "==> [play-shards-report] $TS"
echo "    report: $REPORT_FILE"

declare -A SHARD_EXIT
declare -A SHARD_SUMMARY

overall_exit=0

for shard in "${SHARDS[@]}"; do
    echo ""
    set +e
    bash "$SHARD_SCRIPT" "$shard"
    code=$?
    set -e
    SHARD_EXIT[$shard]=$code
    if (( code == 2 )); then
        overall_exit=2
    elif (( code != 0 )) && (( overall_exit != 2 )); then
        overall_exit=1
    fi
    # Capture summary text (may not exist if shard never ran before)
    SUMMARY_TXT="$SCRIPT_DIR/../tmp/play-artifacts/$shard/summary.txt"
    if [[ -f "$SUMMARY_TXT" ]]; then
        SHARD_SUMMARY[$shard]="$(cat "$SUMMARY_TXT")"
    else
        SHARD_SUMMARY[$shard]="(no summary artifact)"
    fi
done

# Build combined report
{
    echo "shards-report: $TS"
    echo "========================================"
    total_passed=0
    total_failed=0
    total_skipped=0

    for shard in "${SHARDS[@]}"; do
        code="${SHARD_EXIT[$shard]}"
        case "$code" in
            0) status="PASS" ;;
            2) status="BLOCKED" ;;
            *) status="FAIL" ;;
        esac
        echo ""
        echo "--- $shard  [$status exit=$code] ---"

        summary="${SHARD_SUMMARY[$shard]}"
        echo "$summary"

        # Parse counts from summary for totals
        p=$(echo "$summary" | grep '^passed:' | awk '{print $2}')
        f=$(echo "$summary" | grep '^failed:' | awk '{print $2}')
        s=$(echo "$summary" | grep '^skipped:' | awk '{print $2}')
        total_passed=$(( total_passed + ${p:-0} ))
        total_failed=$(( total_failed + ${f:-0} ))
        total_skipped=$(( total_skipped + ${s:-0} ))
    done

    echo ""
    echo "========================================"
    total=$(( total_passed + total_failed + total_skipped ))
    if (( total_failed == 0 )); then
        echo "OVERALL: PASS  $total_passed/$total passed"
    else
        echo "OVERALL: FAIL  $total_passed/$total passed," \
             "$total_failed failed"
    fi
} | tee "$REPORT_FILE"

echo ""
echo "    artifacts: $REPORT_DIR"
exit "$overall_exit"
