#!/bin/bash
# Full RSX System Orchestrator
# Starts all processes in correct order with health checks

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LOG_DIR="$PROJECT_ROOT/log"
PID_DIR="$PROJECT_ROOT/tmp/pids"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Create directories
mkdir -p "$LOG_DIR" "$PID_DIR"

# Configuration
export DATABASE_URL="${DATABASE_URL:-postgresql://postgres:postgres@localhost:5432/rsx_dev}"
export RSX_RISK_SHARD_ID=0
export RSX_RISK_SHARD_COUNT=1
export RSX_RISK_REPLICA=false

# Symbol configurations
declare -A SYMBOLS
SYMBOLS=(
    [0]="BTCUSD"
    [1]="ETHUSD"
    [2]="SOLUSD"
)

echo "=== RSX Full System Startup ==="
echo "Project root: $PROJECT_ROOT"
echo "Logs: $LOG_DIR"
echo "PIDs: $PID_DIR"
echo ""

# Helper: Start process with logging
start_process() {
    local name=$1
    local binary=$2
    shift 2
    local args=("$@")

    local log_file="$LOG_DIR/$name.log"
    local pid_file="$PID_DIR/$name.pid"

    echo -n "Starting $name... "

    # Start process in background
    "$PROJECT_ROOT/target/debug/$binary" "${args[@]}" > "$log_file" 2>&1 &
    local pid=$!
    echo $pid > "$pid_file"

    # Wait a bit and check if still running
    sleep 1
    if kill -0 $pid 2>/dev/null; then
        echo -e "${GREEN}OK${NC} (PID $pid)"
        return 0
    else
        echo -e "${RED}FAILED${NC}"
        echo "Last 10 lines of log:"
        tail -10 "$log_file"
        return 1
    fi
}

# Helper: Check health
check_health() {
    local name=$1
    local check_cmd=$2

    echo -n "Health check $name... "

    local max_attempts=10
    local attempt=0

    while [ $attempt -lt $max_attempts ]; do
        if eval "$check_cmd" > /dev/null 2>&1; then
            echo -e "${GREEN}READY${NC}"
            return 0
        fi
        sleep 1
        ((attempt++))
    done

    echo -e "${RED}TIMEOUT${NC}"
    return 1
}

# Helper: Stop all processes
stop_all() {
    echo ""
    echo "=== Stopping All Processes ==="

    for pid_file in "$PID_DIR"/*.pid; do
        if [ -f "$pid_file" ]; then
            local pid=$(cat "$pid_file")
            local name=$(basename "$pid_file" .pid)

            if kill -0 $pid 2>/dev/null; then
                echo -n "Stopping $name (PID $pid)... "
                kill $pid
                sleep 1

                if kill -0 $pid 2>/dev/null; then
                    kill -9 $pid 2>/dev/null || true
                    echo -e "${YELLOW}KILLED${NC}"
                else
                    echo -e "${GREEN}STOPPED${NC}"
                fi
            fi
            rm -f "$pid_file"
        fi
    done

    echo "All processes stopped"
}

# Trap Ctrl+C
trap stop_all EXIT INT TERM

# Clean old PIDs
rm -f "$PID_DIR"/*.pid

echo "=== Phase 1: Risk Engine ==="
start_process "risk-0" "rsx-risk" || exit 1

echo ""
echo "=== Phase 2: Matching Engines ==="
for sid in "${!SYMBOLS[@]}"; do
    symbol="${SYMBOLS[$sid]}"
    export RSX_ME_SYMBOL_ID=$sid
    export RSX_ME_SYMBOL=$symbol
    start_process "me-$symbol" "rsx-matching" || exit 1
done

echo ""
echo "=== Phase 3: Market Data ==="
start_process "marketdata" "rsx-marketdata" || exit 1

echo ""
echo "=== Phase 4: Mark Price ==="
start_process "mark" "rsx-mark" || exit 1

echo ""
echo "=== Phase 5: Gateway ==="
start_process "gateway" "rsx-gateway" || exit 1
check_health "gateway" "curl -s http://localhost:8080/ > /dev/null" || exit 1

echo ""
echo -e "${GREEN}=== All Processes Running ===${NC}"
echo ""
echo "Process Status:"
for pid_file in "$PID_DIR"/*.pid; do
    if [ -f "$pid_file" ]; then
        local pid=$(cat "$pid_file")
        local name=$(basename "$pid_file" .pid)
        printf "  %-20s PID %-6s %s\n" "$name" "$pid" "$(ps -p $pid -o comm= 2>/dev/null || echo 'DEAD')"
    fi
done

echo ""
echo "Logs available in: $LOG_DIR"
echo "Gateway WebSocket: ws://localhost:8080"
echo ""
echo "Press Ctrl+C to stop all processes"
echo ""

# Keep running until interrupted
wait
