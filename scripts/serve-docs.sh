#!/bin/bash
# Serve MkDocs documentation locally

set -euo pipefail

cd "$(dirname "$0")/.."

VENV_DIR=".mkdocs-venv"

if [ ! -d "$VENV_DIR" ]; then
    echo "Creating virtual environment for MkDocs..."
    if ! python3 -m venv "$VENV_DIR"; then
        echo "ERROR: Failed to create virtual environment."
        echo "On Debian/Ubuntu, install python3-venv:"
        echo "  sudo apt install python3-venv"
        echo ""
        echo "Alternatively, install MkDocs Material globally:"
        echo "  pip install --user mkdocs-material"
        exit 1
    fi
    echo "Installing MkDocs Material..."
    "$VENV_DIR/bin/pip" install --quiet mkdocs-material
fi

if [ ! -f "$VENV_DIR/bin/mkdocs" ]; then
    echo "Installing MkDocs Material..."
    "$VENV_DIR/bin/pip" install --quiet mkdocs-material
fi

echo "Starting MkDocs server on http://0.0.0.0:8001"
echo "Documentation will be available at http://localhost:8001"
echo "Press Ctrl+C to stop"
"$VENV_DIR/bin/mkdocs" serve --dev-addr 0.0.0.0:8001
