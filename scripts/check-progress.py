#!/usr/bin/env python3
"""Validate PROGRESS.md accounting consistency.

Checks that:
1. completed + running + pending + failed == denominator
2. completed / denominator == displayed percentage (within 1%)
3. progress bar numerator == completed count
4. denominator == CANONICAL_TOTAL (223 Playwright tests)

Exit 0 if consistent, 1 if inconsistent (with clear error output).
"""

import re
import sys
from pathlib import Path

PROGRESS_FILE = Path(__file__).parent.parent / "PROGRESS.md"

# Canonical Playwright test count — denominator must equal this.
# Change only when the Playwright suite itself changes.
CANONICAL_TOTAL = 223


def fail(msg: str) -> None:
    print(f"PROGRESS inconsistency: {msg}", file=sys.stderr)
    sys.exit(1)


def main() -> None:
    text = PROGRESS_FILE.read_text()

    # Parse progress bar line: [████░░░] 20%  45/220
    bar_match = re.search(
        r'\[[\█░ ]+\]\s+(\d+)%\s+(\d+)/(\d+)', text
    )
    if not bar_match:
        fail("cannot find progress bar line matching [██░] N%  X/Y")
    pct_displayed = int(bar_match.group(1))
    bar_numerator = int(bar_match.group(2))
    bar_denominator = int(bar_match.group(3))

    # Parse table rows
    def get_count(label: str) -> int:
        m = re.search(rf'\|\s*{label}\s*\|\s*(\d+)\s*\|', text)
        if not m:
            fail(f"cannot find table row for '{label}'")
        return int(m.group(1))

    completed = get_count("completed")
    running = get_count("running")
    pending = get_count("pending")
    failed = get_count("failed")

    total_table = completed + running + pending + failed

    # Check 1: bar numerator == completed
    if bar_numerator != completed:
        fail(
            f"progress bar numerator ({bar_numerator}) != "
            f"completed count ({completed})"
        )

    # Check 2: table sum == denominator
    if total_table != bar_denominator:
        fail(
            f"completed({completed}) + running({running}) + "
            f"pending({pending}) + failed({failed}) = {total_table} "
            f"!= denominator({bar_denominator})"
        )

    # Check 3: displayed percentage matches computed percentage (within 1%)
    if bar_denominator > 0:
        computed_pct = round(100 * completed / bar_denominator)
        if abs(computed_pct - pct_displayed) > 1:
            fail(
                f"displayed {pct_displayed}% != "
                f"computed {computed_pct}% "
                f"({completed}/{bar_denominator})"
            )

    # Check 4: denominator must equal canonical Playwright test count
    if bar_denominator != CANONICAL_TOTAL:
        fail(
            f"denominator ({bar_denominator}) != "
            f"canonical total ({CANONICAL_TOTAL}); "
            f"update PROGRESS.md denominator to {CANONICAL_TOTAL}"
        )

    print(
        f"PROGRESS ok: {completed}/{bar_denominator} "
        f"({pct_displayed}%), "
        f"running={running}, pending={pending}, failed={failed}"
    )


if __name__ == "__main__":
    main()
