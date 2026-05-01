#!/usr/bin/env bash
# bench-gate.sh — Criterion regression gate
# Usage: bash scripts/bench-gate.sh [--save-baseline]
#
# Runs cargo bench --workspace, walks target/criterion/*/new/estimates.json,
# compares against bench-baseline.json (at repo root, checked in), and
# exits 1 if any benchmark exceeds 1.10x baseline.
#
# --save-baseline: overwrite baseline file and exit 0. Commit the result
#   so CI runs have a stable reference.
# No baseline file: print a guidance message and exit 0 (does not fail
#   the build; the project just has no baseline yet).

set -euo pipefail

BASELINE="bench-baseline.json"
SAVE=0

for arg in "$@"; do
    case "$arg" in
        --save-baseline) SAVE=1 ;;
        *) echo "unknown arg: $arg" >&2; exit 1 ;;
    esac
done

# Run benchmarks
echo "==> running cargo bench --workspace"
cargo bench --workspace

# Collect current results
declare -A CURRENT=()
while IFS= read -r f; do
    name=$(echo "$f" \
        | sed 's|target/criterion/||' \
        | sed 's|/new/estimates.json||' \
        | tr '/' '/')
    ns=$(jq '.mean.point_estimate' "$f")
    CURRENT["$name"]="$ns"
done < <(find target/criterion -path '*/new/estimates.json' | sort)

if [ ${#CURRENT[@]} -eq 0 ]; then
    echo "no criterion results found in target/criterion/" >&2
    exit 1
fi

# Save baseline if requested
if [ "$SAVE" -eq 1 ]; then
    printf '{\n' > "$BASELINE"
    first=1
    for name in $(echo "${!CURRENT[@]}" | tr ' ' '\n' | sort); do
        ns="${CURRENT[$name]}"
        if [ "$first" -eq 1 ]; then
            first=0
        else
            printf ',\n' >> "$BASELINE"
        fi
        printf '  "%s": %s' "$name" "$ns" >> "$BASELINE"
    done
    printf '\n}\n' >> "$BASELINE"
    echo "==> baseline saved to $BASELINE (commit this file)"
    exit 0
fi

# No baseline: print guidance and exit clean. The gate is
# only meaningful with a committed reference.
if [ ! -f "$BASELINE" ]; then
    echo "==> no baseline at $BASELINE"
    echo "    create one with: make bench-save"
    echo "    then commit $BASELINE so CI has a reference"
    exit 0
fi

# Compare against baseline
printf "\n%-60s %14s %14s %8s  %s\n" \
    "benchmark" "baseline ns" "current ns" "ratio" "result"
printf '%s\n' "$(printf '%-60s %14s %14s %8s  %s' \
    '------------------------------------------------------------' \
    '--------------' '--------------' '--------' '------')"

FAIL=0

for name in $(echo "${!CURRENT[@]}" | tr ' ' '\n' | sort); do
    current_ns="${CURRENT[$name]}"
    baseline_ns=$(jq --arg n "$name" '.[$n] // empty' "$BASELINE")

    if [ -z "$baseline_ns" ] || [ "$baseline_ns" = "null" ]; then
        printf "%-60s %14s %14.0f %8s  NEW\n" \
            "$name" "n/a" "$current_ns" "-"
        continue
    fi

    # Use awk for float division (bash can't do floats).
    # Guard: if baseline is 0 (should never happen; Criterion min is
    # sub-ns but > 0), treat as NEW rather than dividing by zero.
    if awk "BEGIN { exit ($baseline_ns == 0) ? 0 : 1 }"; then
        printf "%-60s %14s %14.0f %8s  NEW(zero-baseline)\n" \
            "$name" "0" "$current_ns" "-"
        continue
    fi
    ratio=$(awk "BEGIN { printf \"%.4f\", $current_ns / $baseline_ns }")
    pct=$(awk "BEGIN { printf \"%.1f%%\", ($current_ns / $baseline_ns) * 100 }")
    result="PASS"
    fail_flag=$(awk "BEGIN { print ($ratio > 1.10) ? 1 : 0 }")
    if [ "$fail_flag" -eq 1 ]; then
        result="FAIL"
        FAIL=1
    fi

    printf "%-60s %14.0f %14.0f %8s  %s\n" \
        "$name" "$baseline_ns" "$current_ns" "$pct" "$result"
done

echo ""
if [ "$FAIL" -ne 0 ]; then
    echo "==> FAIL: one or more benchmarks regressed >10%"
    exit 1
fi
echo "==> PASS: all benchmarks within 10% of baseline"
exit 0
