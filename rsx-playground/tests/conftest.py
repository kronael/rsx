"""Pytest configuration for rsx-playground tests."""

import sys
from pathlib import Path

# Add parent directory to path for server imports
sys.path.insert(0, str(Path(__file__).parent.parent))
