#!/usr/bin/env bash
# smoke.sh -- prove the already-deployed RSX stack can complete one trade.

set -Eeuo pipefail

PLAYGROUND_URL=${PLAYGROUND_URL:-${RSX_URL:-http://localhost:49171}}
export PLAYGROUND_URL
export DEMO_START_CLUSTER=0

printf '%s INFO smoke: checking deployed stack at %s\n' \
    "$(date '+%b %d %H:%M:%S')" "$PLAYGROUND_URL"

exec bash scripts/demo-trade.sh
