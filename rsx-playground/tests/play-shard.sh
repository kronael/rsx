#!/usr/bin/env bash
# play-shard.sh - Run a Playwright domain shard with failure-signature hashing.
#
# Usage: ./play-shard.sh <shard-name>
#   shard-name: routing | htmx-partials | process-control | trade-ui
#
# Failure-signature hashing:
#   - After a failed run, the set of failing test titles is hashed and stored
#     in tmp/play-sig/<shard>.sig
#   - On re-run, if the signature matches (same failures, no code diff in
#     domain files), the run is blocked and exits 2 (no-op / already known)
#   - If the signature changed OR git diff touches domain files, run proceeds
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
SIG_DIR="$SCRIPT_DIR/../tmp/play-sig"
mkdir -p "$SIG_DIR"
SIG_FILE="$SIG_DIR/${SHARD}.sig"
OUT_FILE="$SIG_DIR/${SHARD}.out"

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
    # Check if any tracked changes touch domain files
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

# Run playwright for this shard (project name matches shard name)
run_shard() {
    cd "$SCRIPT_DIR"
    npx playwright test --project="$SHARD" --reporter=json 2>/dev/null \
        > "$OUT_FILE" || true
}

# Extract sorted list of failing test titles from JSON output → hash
failure_sig() {
    if [[ ! -f "$OUT_FILE" ]]; then
        echo "no-output"
        return
    fi
    python3 - "$OUT_FILE" <<'EOF'
import json, sys, hashlib
data = json.load(open(sys.argv[1]))
failures = []
for suite in data.get("suites", []):
    for spec in suite.get("specs", []):
        for test in spec.get("tests", []):
            if any(r.get("status") == "failed" for r in test.get("results", [])):
                failures.append(spec.get("title", "") + "::" + test.get("title", ""))
failures.sort()
digest = hashlib.sha256("\n".join(failures).encode()).hexdigest()[:16]
print(digest if failures else "pass")
EOF
}

# Check if run passed
run_passed() {
    if [[ ! -f "$OUT_FILE" ]]; then
        return 1
    fi
    python3 - "$OUT_FILE" <<'EOF'
import json, sys
data = json.load(open(sys.argv[1]))
total = data.get("stats", {})
sys.exit(0 if total.get("unexpected", 0) == 0 else 1)
EOF
}

echo "==> [play-shard] $SHARD"

run_shard

if run_passed; then
    echo "    PASS: $SHARD"
    rm -f "$SIG_FILE"
    exit 0
fi

# Compute current failure signature
CURRENT_SIG="$(failure_sig)"
echo "    FAIL: signature=$CURRENT_SIG"

if [[ -f "$SIG_FILE" ]]; then
    PREV_SIG="$(cat "$SIG_FILE")"
    if [[ "$CURRENT_SIG" == "$PREV_SIG" ]]; then
        # Same failures — block re-run unless domain files changed
        if domain_changed; then
            echo "    domain files changed — re-run allowed (same sig, new code)"
            echo "$CURRENT_SIG" > "$SIG_FILE"
            exit 1
        else
            echo "    BLOCKED: same failure signature, no domain changes"
            echo "    Fix the failing tests before re-running this shard."
            exit 2
        fi
    fi
fi

# New or changed signature — record and fail
echo "$CURRENT_SIG" > "$SIG_FILE"
exit 1
