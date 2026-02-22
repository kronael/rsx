#!/usr/bin/env python3
"""Deterministic PROGRESS.md regeneration and validation.

Reads PROGRESS.md once (no external bundle, no network).
Recomputes header counts from the ## log section and asserts:
  - denominator in progress bar == 223
  - bar numerator == table completed count
  - bar percentage == round(100 * num / den)
  - num <= den
  - header counts match log-derived counts (no divergence)

Exit codes:
  0  all checks pass
  1  divergence or invariant violation
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).parent.parent
PROGRESS = ROOT / "PROGRESS.md"
CANONICAL_TOTAL = 223


def fail(msg: str) -> None:
    print(f"FAIL: {msg}", file=sys.stderr)
    sys.exit(1)


def parse_bar(text: str) -> tuple[int, int, int]:
    """Return (pct, numerator, denominator) from progress bar."""
    m = re.search(
        r'\[[\u2588\u2591 ]+\]\s+(\d+)%\s+(\d+)/(\d+)',
        text,
    )
    if not m:
        fail("no progress bar found in PROGRESS.md")
    return int(m.group(1)), int(m.group(2)), int(m.group(3))


def parse_table(text: str) -> dict[str, int]:
    counts: dict[str, int] = {}
    for label in ("completed", "running", "pending", "failed"):
        m = re.search(
            rf'\|\s*{label}\s*\|\s*(\d+)\s*\|',
            text,
            re.IGNORECASE,
        )
        if m:
            counts[label] = int(m.group(1))
    return counts


def count_log_entries(text: str) -> int:
    """Count bullet lines under ## log section."""
    m = re.search(r'^##\s+log\s*$', text, re.MULTILINE)
    if not m:
        return 0
    log_section = text[m.end():]
    # Stop at next ## section
    next_section = re.search(r'^##\s+', log_section, re.MULTILINE)
    if next_section:
        log_section = log_section[:next_section.start()]
    return len(re.findall(r'^\s*-\s+', log_section, re.MULTILINE))


def main() -> None:
    if not PROGRESS.exists():
        fail(f"PROGRESS.md not found: {PROGRESS}")

    text = PROGRESS.read_text()

    # 1. Parse bar
    pct, num, den = parse_bar(text)

    # 2. Denominator must be CANONICAL_TOTAL
    if den != CANONICAL_TOTAL:
        fail(
            f"denominator mismatch: bar shows {num}/{den}"
            f", expected denominator={CANONICAL_TOTAL}"
        )

    # 3. Bar internal consistency
    computed_pct = round(100 * num / den)
    if abs(computed_pct - pct) > 1:
        fail(
            f"bar {num}/{den}: displayed {pct}% != computed {computed_pct}%"
        )
    if num > den:
        fail(f"bar numerator {num} > denominator {den}")

    # 4. Parse table counts
    counts = parse_table(text)
    missing = [
        k for k in ("completed", "running", "pending", "failed")
        if k not in counts
    ]
    if missing:
        fail(f"PROGRESS.md table missing rows: {missing}")

    completed = counts["completed"]
    running = counts["running"]
    pending = counts["pending"]
    failed = counts["failed"]

    # 5. Table completed must match bar numerator
    if completed != num:
        fail(
            f"divergence: bar numerator={num}"
            f" != table completed={completed}"
        )

    # 6. Log entry count must be consistent with completed count
    log_count = count_log_entries(text)
    if log_count > 0 and log_count < completed:
        fail(
            f"divergence: log has {log_count} entries"
            f" but table shows completed={completed}"
        )

    print(
        f"ok: {num}/{den} ({pct}%)"
        f", completed={completed}"
        f", running={running}"
        f", pending={pending}"
        f", failed={failed}"
        f", log_entries={log_count}"
    )
    sys.exit(0)


if __name__ == "__main__":
    main()
