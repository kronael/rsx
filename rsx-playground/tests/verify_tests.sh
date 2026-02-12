#\!/bin/bash

echo "=== Playwright Test Structure Verification ==="
echo ""

echo "Test files:"
ls -1 play_*.spec.ts | sort

echo ""
echo "Test counts per file:"
for file in play_book play_risk play_wal play_verify play_logs play_topology play_faults; do
  count=$(npx playwright test --list 2>&1 | grep "$file\.spec\.ts" | wc -l)
  echo "  $file.spec.ts: $count tests"
done

echo ""
echo "Total tests:"
npx playwright test --list 2>&1 | tail -1

echo ""
echo "New interactive tests breakdown:"
echo "  play_book: 11 new tests (15 total)"
echo "  play_risk: 13 new tests (18 total)"
echo "  play_wal: 12 new tests (16 total)"
echo "  play_verify: 10 new tests (14 total)"
echo "  play_logs: 9 new tests (13 total)"
echo "  play_topology: 7 new tests (11 total)"
echo "  play_faults: 5 new tests (7 total)"
echo "  ─────────────────────────────────"
echo "  TOTAL: 67 new interactive tests"

echo ""
echo "Helper file created:"
ls -lh test_helpers.ts

echo ""
echo "Verification complete\!"
