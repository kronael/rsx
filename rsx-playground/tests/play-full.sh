#!/usr/bin/env bash
# play-full.sh - Run the full Playwright suite; publish timestamped artifacts.
#
# One monolithic execution — all projects — with JSON + JUnit reporters.
# Artifacts are the only accepted proof for pass claims.
#
# Strategy: set PW_SHARD to the timestamped run name so playwright.config.ts
# activates the json+junit reporters with paths under that directory.
# The canonical full-run/ location is updated after the run completes.
#
# Artifacts written to:
#   tmp/play-artifacts/run-<YYYYMMDD_HHMMSS>/report.json
#   tmp/play-artifacts/run-<YYYYMMDD_HHMMSS>/report.xml
#   tmp/play-artifacts/run-<YYYYMMDD_HHMMSS>/run.log
#   tmp/play-artifacts/full-run/report.json   (latest canonical)
#   tmp/play-artifacts/full-run/report.xml
#
# Exit codes:
#   0  all tests passed
#   1  one or more tests failed

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TS="$(date +%Y%m%d_%H%M%S)"
RUN_SHARD="run-$TS"
RUN_DIR="$SCRIPT_DIR/../tmp/play-artifacts/$RUN_SHARD"
FULL_DIR="$SCRIPT_DIR/../tmp/play-artifacts/full-run"

mkdir -p "$RUN_DIR" "$FULL_DIR"

echo "==> [play-full] $TS"
echo "    artifacts: $RUN_DIR"

cd "$SCRIPT_DIR"

# PW_SHARD activates json+junit reporters in playwright.config.ts and sets
# artifactDir to tmp/play-artifacts/<shard>/.  No --project flag: all projects
# run in dependency order as defined in playwright.config.ts.
set +e
PW_SHARD="$RUN_SHARD" bunx playwright test 2>&1 | tee "$RUN_DIR/run.log"
EXIT="${PIPESTATUS[0]}"
set -e

# Verify artifacts were written
if [[ ! -f "$RUN_DIR/report.json" ]]; then
    echo "    ERROR: report.json not written to $RUN_DIR" >&2
    exit 1
fi

# Publish to canonical full-run location (overwrite — latest run wins).
cp "$RUN_DIR/report.json" "$FULL_DIR/report.json"
cp "$RUN_DIR/report.xml"  "$FULL_DIR/report.xml"

# Parse summary
python3 - "$RUN_DIR/report.json" <<'PYEOF'
import json, sys
data = json.load(open(sys.argv[1]))
s = data.get("stats", {})
passed  = s.get("expected",   0)
failed  = s.get("unexpected", 0)
skipped = s.get("skipped",    0)
total   = passed + failed + skipped
status  = "PASS" if failed == 0 else "FAIL"
print(f"    {status}: {passed}/{total} passed"
      + (f", {failed} failed" if failed else "")
      + (f", {skipped} skipped" if skipped else ""))
PYEOF

echo "    canonical: $FULL_DIR"
exit "$EXIT"
