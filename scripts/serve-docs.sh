#!/bin/bash
# Serve MkDocs documentation locally

set -euo pipefail

cd "$(dirname "$0")/.."

if ! command -v mkdocs &> /dev/null; then
    echo "MkDocs not installed. Installing with pip..."
    pip install mkdocs-material
fi

echo "Starting MkDocs server on http://0.0.0.0:8001"
echo "Press Ctrl+C to stop"
mkdocs serve --dev-addr 0.0.0.0:8001
