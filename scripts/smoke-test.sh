#!/bin/bash
# Smoke Test: 100 orders/sec for 60 seconds
# Phase 2 of validation plan

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m'

echo "=== RSX Smoke Test ==="
echo "Target: 100 orders/sec for 60 seconds (6,000 orders total)"
echo ""

# Check if system is running
if ! curl -s http://localhost:8080/ > /dev/null 2>&1; then
    echo -e "${RED}ERROR: Gateway not running${NC}"
    echo "Start system with: ./scripts/run-full-system.sh"
    exit 1
fi

echo -e "${GREEN}Gateway detected${NC}"
echo ""

# Run stress test
echo "Starting stress test..."
"$PROJECT_ROOT/target/release/rsx-stress" \
    --gateway ws://localhost:8080 \
    --rate 100 \
    --duration 60 \
    --symbols BTCUSD \
    --users 10 \
    --connections 5 \
    --output smoke-test.csv

echo ""
echo "=== Smoke Test Complete ==="
echo "Results: smoke-test.csv"
echo ""

# Basic validation
if [ -f smoke-test.csv ]; then
    line_count=$(wc -l < smoke-test.csv)
    echo "CSV rows: $line_count"

    if [ "$line_count" -gt 5000 ]; then
        echo -e "${GREEN}PASS: Submitted >5,000 orders${NC}"
    else
        echo -e "${RED}FAIL: Only $line_count orders${NC}"
    fi
fi
