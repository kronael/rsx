#!/usr/bin/env bash
# e2e-matrix.sh — Live E2E matrix across all 13 pages, grouped by flow step.
#
# Flow steps:
#   startup        gate-1: server.py imports cleanly
#   routing        test_htmx_partials + test_no_absolute_links
#   htmx-partials  api_processes, api_risk, api_wal, api_orders,
#                  api_logs_metrics, api_verify
#   proxy          api_proxy_test
#   spa-assets     api_e2e, api_edge_cases
#   order-path     api_integration
#
# Usage: ./e2e-matrix.sh [--json]
#   --json   emit machine-readable JSON to stdout
#
# Artifacts: tmp/e2e-matrix/<step>/run.log
# Exit:  0 all green, 1 some failures, 2 hard error

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PLAYGROUND="$SCRIPT_DIR/.."
PYTEST="$PLAYGROUND/.venv/bin/pytest"
PY="$PLAYGROUND/.venv/bin/python3"
ARTIFACT_BASE="$PLAYGROUND/tmp/e2e-matrix"
JSON_MODE=0
[[ "${1:-}" == "--json" ]] && JSON_MODE=1

mkdir -p "$ARTIFACT_BASE"

# ── Per-step result accumulator ───────────────────────────────────────
declare -a STEPS=()
declare -A STEP_STATUS=()
declare -A STEP_PASSED=()
declare -A STEP_FAILED=()
declare -A STEP_IDS=()

OVERALL=0

# ── Helpers ───────────────────────────────────────────────────────────

record_step() {
    local step="$1" status="$2" passed="$3" failed="$4" ids="$5"
    STEPS+=("$step")
    STEP_STATUS[$step]="$status"
    STEP_PASSED[$step]="$passed"
    STEP_FAILED[$step]="$failed"
    STEP_IDS[$step]="$ids"
}

parse_pytest_log() {
    # Parse pytest -v output for PASSED/FAILED counts and IDs
    local log="$1"
    local passed=0 failed=0 ids=""
    passed=$(grep -c ' PASSED' "$log" 2>/dev/null || true)
    failed=$(grep -c ' FAILED' "$log" 2>/dev/null || true)
    ids=$(grep ' FAILED' "$log" 2>/dev/null \
        | sed 's/ FAILED.*//' \
        | sed 's/^FAILED //' \
        | sed 's/^ *//' \
        | sort -u \
        | tr '\n' ',' \
        | sed 's/,$//' || true)
    echo "$passed $failed $ids"
}

run_pytest_step() {
    local step="$1"
    local label="$2"
    shift 2
    local out_dir="$ARTIFACT_BASE/$step"
    mkdir -p "$out_dir"
    local log="$out_dir/run.log"

    [[ $JSON_MODE -eq 0 ]] && echo "==> [$step] $label"

    local rc=0
    "$PYTEST" "$@" --tb=short -v >"$log" 2>&1 || rc=$?

    read -r passed failed ids < <(parse_pytest_log "$log")
    passed=${passed:-0}
    failed=${failed:-0}
    ids=${ids:-}

    if [[ $rc -eq 0 ]]; then
        record_step "$step" "PASS" "$passed" "0" ""
        [[ $JSON_MODE -eq 0 ]] && echo "    PASS ($passed passed)"
    else
        record_step "$step" "FAIL" "$passed" "$failed" "$ids"
        OVERALL=1
        [[ $JSON_MODE -eq 0 ]] && {
            echo "    FAIL ($failed failed, $passed passed)"
            if [[ -n "$ids" ]]; then
                echo "$ids" | tr ',' '\n' | sed 's/^/      FAILED: /'
            fi
            # Show short tracebacks
            grep -A3 'AssertionError\|FAILED\|Error:' "$log" \
                | head -20 | sed 's/^/      /' || true
        }
    fi
    return $rc
}

run_cmd_step() {
    local step="$1"
    local label="$2"
    shift 2
    local out_dir="$ARTIFACT_BASE/$step"
    mkdir -p "$out_dir"
    local log="$out_dir/run.log"

    [[ $JSON_MODE -eq 0 ]] && echo "==> [$step] $label"

    local rc=0
    "$@" >"$log" 2>&1 || rc=$?

    if [[ $rc -eq 0 ]]; then
        record_step "$step" "PASS" "1" "0" ""
        [[ $JSON_MODE -eq 0 ]] && echo "    PASS"
    else
        local err
        err=$(tail -3 "$log" | tr '\n' ' ')
        record_step "$step" "FAIL" "0" "1" "$step::startup"
        OVERALL=1
        [[ $JSON_MODE -eq 0 ]] && echo "    FAIL: $err"
    fi
    return $rc
}

# ── Flow Steps ────────────────────────────────────────────────────────

# Step 1: startup
run_cmd_step "startup" "server.py imports cleanly" \
    "$PY" -c \
    "import sys; sys.path.insert(0,'$PLAYGROUND'); import server; print('ok')"

# Step 2: routing — page routes + HTMX partials + no absolute links
run_pytest_step "routing" "all 13 pages + 38 HTMX partials HTTP 200" \
    "$PLAYGROUND/tests/test_htmx_partials.py" \
    "$PLAYGROUND/tests/test_no_absolute_links.py"

# Step 3: htmx-partials — API endpoints backing the data pages
run_pytest_step "htmx-partials" \
    "API: processes, risk, WAL, orders, logs, verify" \
    "$PLAYGROUND/tests/api_processes_test.py" \
    "$PLAYGROUND/tests/api_risk_test.py" \
    "$PLAYGROUND/tests/api_wal_test.py" \
    "$PLAYGROUND/tests/api_orders_test.py" \
    "$PLAYGROUND/tests/api_logs_metrics_test.py" \
    "$PLAYGROUND/tests/api_verify_test.py"

# Step 4: proxy — WebSocket + REST proxy routes
run_pytest_step "proxy" "WebSocket + REST proxy endpoints" \
    "$PLAYGROUND/tests/api_proxy_test.py"

# Step 5: spa-assets — API edge cases (covers SPA/trade page deps)
run_pytest_step "spa-assets" "API edge cases + e2e flow" \
    "$PLAYGROUND/tests/api_e2e_test.py" \
    "$PLAYGROUND/tests/api_edge_cases_test.py"

# Step 6: order-path — full order lifecycle integration
run_pytest_step "order-path" "order lifecycle integration" \
    "$PLAYGROUND/tests/api_integration_test.py"

# ── Emit report ───────────────────────────────────────────────────────

if [[ $JSON_MODE -eq 1 ]]; then
    # Build JSON from accumulated results
    first=1
    echo "{"
    if [[ $OVERALL -eq 0 ]]; then
        echo '  "overall": "PASS",'
    else
        echo '  "overall": "FAIL",'
    fi
    echo '  "steps": ['
    for step in "${STEPS[@]}"; do
        [[ $first -eq 0 ]] && echo "    ,"
        first=0
        status="${STEP_STATUS[$step]}"
        passed="${STEP_PASSED[$step]}"
        failed="${STEP_FAILED[$step]}"
        ids="${STEP_IDS[$step]}"
        # Build JSON array of failing IDs
        ids_json="[]"
        if [[ -n "$ids" ]]; then
            ids_json="[$(echo "$ids" | tr ',' '\n' \
                | sed 's/.*/"&"/' | tr '\n' ',' | sed 's/,$//')]"
        fi
        printf '    {"step":"%s","status":"%s","passed":%s,"failed":%s,"failing_ids":%s}\n' \
            "$step" "$status" "$passed" "$failed" "$ids_json"
    done
    echo "  ]"
    echo "}"
else
    echo ""
    echo "── E2E Matrix Results ──────────────────────────────────────"
    printf "%-20s %-6s %6s %6s\n" "STEP" "STATUS" "PASS" "FAIL"
    printf "%-20s %-6s %6s %6s\n" "----" "------" "----" "----"
    for step in "${STEPS[@]}"; do
        printf "%-20s %-6s %6s %6s\n" \
            "$step" \
            "${STEP_STATUS[$step]}" \
            "${STEP_PASSED[$step]}" \
            "${STEP_FAILED[$step]}"
        ids="${STEP_IDS[$step]}"
        if [[ -n "$ids" && "${STEP_STATUS[$step]}" == "FAIL" ]]; then
            echo "$ids" | tr ',' '\n' | sed 's/^/    FAILED: /'
        fi
    done
    echo "────────────────────────────────────────────────────────────"
    if [[ $OVERALL -eq 0 ]]; then
        echo "OVERALL: PASS"
    else
        echo "OVERALL: FAIL — fix above failures before gate-4 Playwright"
    fi
fi

exit $OVERALL
