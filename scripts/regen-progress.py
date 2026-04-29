#!/usr/bin/env python3
"""Compatibility wrapper for canonical PROGRESS regeneration.

Historical versions maintained a second PROGRESS accounting model here.
That created drift against publish-progress.py. Keep one generator only:
delegate to publish-progress.py.
"""

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).parent.parent
PUBLISH = ROOT / "scripts" / "publish-progress.py"


def main() -> None:
    result = subprocess.run(
        [sys.executable, str(PUBLISH), *sys.argv[1:]],
        cwd=ROOT,
    )
    sys.exit(result.returncode)


if __name__ == "__main__":
    main()
