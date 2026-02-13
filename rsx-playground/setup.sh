#!/bin/bash
# RSX Playground Setup Script
# Ensures all dependencies and processes are ready

set -e

cd "$(dirname "$0")"

# Set uv cache to writable location
export UV_CACHE_DIR="$(pwd)/../tmp/.uv-cache"
mkdir -p "$UV_CACHE_DIR"

echo "=== RSX Playground Setup ==="

# Check Python version
echo "1. Checking Python..."
python3 --version || { echo "Python 3 not found"; exit 1; }

# Check uv available
echo "2. Checking uv..."
command -v uv &> /dev/null || { echo "ERROR: uv not found. Install from: https://github.com/astral-sh/uv"; exit 1; }
echo "   ✓ uv available"

# Install dependencies
echo "3. Installing dependencies..."
echo "   Using uv (cache: $UV_CACHE_DIR)..."
uv pip install -r requirements.txt

# Verify imports
echo "3. Verifying dependencies..."
python3 -c "import psutil, httpx, websockets; print('   ✓ All dependencies available')" || {
    echo "   ERROR: Dependencies not properly installed"
    exit 1
}

# Verify syntax
echo "4. Checking Python syntax..."
python3 -m py_compile server.py pages.py stress_client.py
echo "   ✓ Syntax valid"

# Create required directories
echo "5. Creating directories..."
mkdir -p ../tmp/stress-reports
mkdir -p ../tmp/wal
mkdir -p ../log
echo "   ✓ Directories created"

# Build RSX binaries if needed
echo "6. Checking RSX binaries..."
if [ ! -f ../target/debug/rsx-gateway ]; then
    echo "   Building rsx-gateway..."
    cd .. && cargo build -p rsx-gateway && cd rsx-playground
fi
echo "   ✓ Gateway binary exists"

# Check if Gateway is running
echo "7. Checking Gateway status..."
if ! pgrep -f rsx-gateway > /dev/null; then
    echo "   Gateway not running. Start it with: ../target/debug/rsx-gateway &"
    echo "   (or use ../start minimal)"
else
    echo "   ✓ Gateway is running"
fi

echo ""
echo "=== Setup Complete ==="
echo ""
echo "Start playground:"
echo "  python3 server.py"
echo ""
echo "Or with uv:"
echo "  uv run server.py"
echo ""
echo "Then visit: http://localhost:49171"
echo ""
