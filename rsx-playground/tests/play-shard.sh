#!/usr/bin/env bash
# play-shard.sh - Run a Playwright domain shard with artifact-based reporting.
#
# Usage: ./play-shard.sh <shard-name>
#   shard-name: routing | htmx-partials | process-control | trade-ui
#
# Artifacts written to tmp/play-artifacts/<shard>/:
#   report.json   Playwright JSON reporter output
#   report.xml    JUnit XML output
#   summary.txt   Human-readable pass/fail counts + failing test IDs
#   sig.txt       SHA-256[:16] of sorted failing test IDs (signature)
#
# Signature-based single retry policy:
#   - On failure, signature is stored in tmp/play-sig/<shard>.sig
#   - Re-run is BLOCKED (exit 2) if same signature AND no domain code change
#   - Re-run proceeds if signature changed OR domain files changed
#
# Exit codes:
#   0  all tests passed
#   1  tests failed (new or changed failures)
#   2  blocked: same failure signature, no domain code changes

set -euo pipefail

SHARD="${1:-}"
if [[ -z "$SHARD" ]]; then
    echo "usage: $0 <routing|htmx-partials|process-control|trade-ui>" >&2
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ARTIFACT_DIR="$SCRIPT_DIR/../tmp/play-artifacts/$SHARD"
SIG_DIR="$SCRIPT_DIR/../tmp/play-sig"
mkdir -p "$ARTIFACT_DIR" "$SIG_DIR"

JSON_OUT="$ARTIFACT_DIR/report.json"
JUNIT_OUT="$ARTIFACT_DIR/report.xml"
SUMMARY_OUT="$ARTIFACT_DIR/summary.txt"
SIG_FILE="$SIG_DIR/${SHARD}.sig"

# Static shard manifest — must match playwright.config.ts projects
declare -A SHARD_SPECS
SHARD_SPECS[routing]="play_navigation.spec.ts play_overview.spec.ts play_topology.spec.ts"
SHARD_SPECS[htmx-partials]="play_book.spec.ts play_risk.spec.ts play_wal.spec.ts play_logs.spec.ts play_faults.spec.ts play_verify.spec.ts"
SHARD_SPECS[process-control]="play_control.spec.ts play_orders.spec.ts"
SHARD_SPECS[trade-ui]="play_trade.spec.ts"

# Domain → source files that, if changed, force a re-run
declare -A DOMAIN_FILES
DOMAIN_FILES[routing]="rsx-playground/server.py rsx-playground/pages.py"
DOMAIN_FILES[htmx-partials]="rsx-playground/server.py rsx-playground/pages.py"
DOMAIN_FILES[process-control]="rsx-playground/server.py rsx-playground/pages.py"
DOMAIN_FILES[trade-ui]="rsx-webui/src"

domain_changed() {
    local files="${DOMAIN_FILES[$SHARD]:-}"
    if [[ -z "$files" ]]; then
        return 0  # unknown shard → always run
    fi
    for f in $files; do
        if git diff --name-only HEAD 2>/dev/null | grep -q "^$f" 2>/dev/null; then
            return 0
        fi
        if git diff --name-only --cached 2>/dev/null | grep -q "^$f" 2>/dev/null; then
            return 0
        fi
    done
    return 1
}

# Run playwright for this shard — write both JSON and JUnit artifacts.
# PW_SHARD env var tells playwright.config.ts to use json+junit reporters
# with artifact paths under tmp/play-artifacts/<shard>/.
run_shard() {
    cd "$SCRIPT_DIR"
    mkdir -p "$ARTIFACT_DIR"
    PW_SHARD="$SHARD" npx playwright test \
        --project="$SHARD" \
        2>/tmp/play-shard-stderr.log || true
}

# Parse JSON artifact — compute: passed, failed, total, sorted failing IDs
parse_artifacts() {
    python3 - "$JSON_OUT" "$SUMMARY_OUT" <<'EOF'
import json
import sys
import hashlib

json_path = sys.argv[1]
summary_path = sys.argv[2]

try:
    data = json.load(open(json_path))
except Exception as e:
    print(f"ERROR: cannot parse {json_path}: {e}", file=sys.stderr)
    sys.exit(1)

stats = data.get("stats", {})
passed = stats.get("expected", 0)
failed = stats.get("unexpected", 0)
skipped = stats.get("skipped", 0)
total = passed + failed + skipped

# Collect failing test IDs: "file::title"
failing = []
def walk_suites(suites):
    for suite in suites:
        for spec in suite.get("specs", []):
            for test in spec.get("tests", []):
                results = test.get("results", [])
                if any(r.get("status") == "failed" for r in results):
                    failing.append(
                        spec.get("file", "?") + "::" + spec.get("title", "?")
                    )
        walk_suites(suite.get("suites", []))

walk_suites(data.get("suites", []))
failing.sort()

# Signature
sig_input = "\n".join(failing)
sig = hashlib.sha256(sig_input.encode()).hexdigest()[:16] if failing else "pass"

# Write summary artifact
lines = [
    f"shard:   {json_path.split('/')[-3] if '/' in json_path else '?'}",
    f"total:   {total}",
    f"passed:  {passed}",
    f"failed:  {failed}",
    f"skipped: {skipped}",
    f"sig:     {sig}",
    "",
]
if failing:
    lines.append("failing tests:")
    for f in failing:
        lines.append(f"  FAIL  {f}")
else:
    lines.append("failing tests: none")

summary_text = "\n".join(lines) + "\n"
open(summary_path, "w").write(summary_text)
print(summary_text, end="")

# Write sig to stdout last line for shell capture
print(f"__SIG__={sig}")
EOF
}

echo "==> [play-shard] $SHARD"

run_shard

# Parse artifacts and capture sig
PARSE_OUT="$(parse_artifacts 2>&1)"
echo "$PARSE_OUT" | grep -v '^__SIG__=' || true
CURRENT_SIG="$(echo "$PARSE_OUT" | grep '^__SIG__=' | cut -d= -f2)"

if [[ -z "$CURRENT_SIG" ]]; then
    echo "    ERROR: could not parse shard artifacts" >&2
    exit 1
fi

# Write sig to artifact dir too
echo "$CURRENT_SIG" > "$ARTIFACT_DIR/sig.txt"

if [[ "$CURRENT_SIG" == "pass" ]]; then
    echo "    PASS: $SHARD (artifacts: $ARTIFACT_DIR)"
    rm -f "$SIG_FILE"
    exit 0
fi

echo "    FAIL: $SHARD  sig=$CURRENT_SIG"
echo "    artifacts: $ARTIFACT_DIR"

# Signature-based single retry policy
if [[ -f "$SIG_FILE" ]]; then
    PREV_SIG="$(cat "$SIG_FILE")"
    if [[ "$CURRENT_SIG" == "$PREV_SIG" ]]; then
        if domain_changed; then
            echo "    domain files changed — retry allowed (same sig, new code)"
            echo "$CURRENT_SIG" > "$SIG_FILE"
            exit 1
        else
            echo "    BLOCKED: same failure signature, no domain changes"
            echo "    Fix the failing tests before re-running this shard."
            echo "    See: $SUMMARY_OUT"
            exit 2
        fi
    fi
fi

# New or changed signature — record and fail
echo "$CURRENT_SIG" > "$SIG_FILE"
exit 1
