#!/usr/bin/env python3
"""Validate PROGRESS.md accounting and gate-3 report consistency.

Checks:
1. completed + running + pending + failed == denominator
2. completed / denominator == displayed percentage (within 1%)
3. progress bar numerator == completed count
4. denominator == CANONICAL_TOTAL (223 Playwright tests)
5. gate-3-report.json: no test key in both DONE and FAIL sets
   (contradiction linter — rejects snapshots with split outcome)

Exit 0 if consistent, 1 if inconsistent (with clear error output).
"""

import json
import re
import sys
from pathlib import Path

PROGRESS_FILE = Path(__file__).parent.parent / "PROGRESS.md"
REPORT_FILE = (
    Path(__file__).parent.parent / "rsx-playground" / "tmp"
    / "gate-3-report.json"
)
PLAY_ARTIFACT_DIR = (
    Path(__file__).parent.parent / "rsx-playground" / "tmp"
    / "play-artifacts"
)
PLAY_SHARDS = ["routing", "htmx-partials", "process-control", "trade-ui"]

# Canonical Playwright test count — denominator must equal this.
# Change only when the Playwright suite itself changes.
CANONICAL_TOTAL = 223


def fail(msg: str) -> None:
    print(f"PROGRESS inconsistency: {msg}", file=sys.stderr)
    sys.exit(1)


def lint_report(path: Path) -> None:
    """Contradiction linter for gate-3-report.json snapshots.

    Rejects any snapshot where a test key appears in both the DONE
    set (outcome=passed) and the FAIL/retry set (listed in failures[])
    in the same update.  Also catches duplicate test entries with
    conflicting outcomes across endpoint classes.
    """
    if not path.exists():
        return  # no report yet — skip silently

    try:
        data = json.loads(path.read_text())
    except Exception as e:
        fail(f"gate-3-report.json parse error: {e}")

    by_class = data.get("by_class", {})

    # Build: failed_tests = {test_id: endpoint_class}
    failed_tests: dict[str, str] = {}
    for ec, info in by_class.items():
        for entry in info.get("failures", []):
            tid = entry.get("test", "")
            if tid in failed_tests:
                fail(
                    f"contradiction: test appears in FAIL set of two "
                    f"endpoint classes: '{tid}' "
                    f"('{failed_tests[tid]}' and '{ec}')"
                )
            failed_tests[tid] = ec

    # Build: passed_tests = {test_id: endpoint_class}
    # The report stores pass/fail counts per class but not individual
    # passed test IDs.  We infer from the raw test list embedded in
    # the "failures" entries vs the total counts.
    #
    # For contradiction detection we use the full_results list if
    # present (written by extended conftest), otherwise we can only
    # check for tests that appear in failures of multiple classes.
    full_results = data.get("results", [])
    if full_results:
        # Check each test's last outcome against the failures set
        # A test is DONE if its final outcome == "passed"
        # A test is FAIL if it appears in any failures list
        seen: dict[str, str] = {}  # test_id -> last outcome
        for entry in full_results:
            tid = entry.get("test", "")
            outcome = entry.get("outcome", "")
            if tid in seen and seen[tid] != outcome:
                # Same test recorded with two different outcomes
                if tid in failed_tests:
                    fail(
                        f"contradiction: '{tid}' is in DONE set "
                        f"(outcome=passed) AND FAIL set "
                        f"(endpoint_class='{failed_tests[tid]}')"
                    )
            seen[tid] = outcome

        # Any test in failures that is also seen as passed = contradiction
        for tid, ec in failed_tests.items():
            if seen.get(tid) == "passed":
                fail(
                    f"contradiction: '{tid}' appears as passed in "
                    f"results[] AND as failed in by_class['{ec}']"
                )

    print(
        f"gate-3-report ok: {len(failed_tests)} failed, "
        f"no contradictions"
    )


def lint_playwright_artifacts(completed: int) -> None:
    """Cross-validate Playwright shard artifacts vs PROGRESS.md completed.

    When shard artifacts exist, the total passing count across all shards
    must equal the PROGRESS.md completed count.  Skips silently when no
    artifacts are present (shards not yet run).
    """
    total_passed = 0
    total_failed = 0
    shards_present: list[str] = []

    for shard in PLAY_SHARDS:
        report_file = PLAY_ARTIFACT_DIR / shard / "report.json"
        if not report_file.exists():
            continue
        try:
            data = json.loads(report_file.read_text())
        except Exception as e:
            fail(f"play-artifacts/{shard}/report.json parse error: {e}")
        stats = data.get("stats", {})
        total_passed += stats.get("expected", 0)
        total_failed += stats.get("unexpected", 0)
        shards_present.append(shard)

    if not shards_present:
        print("playwright artifacts: not yet run (skip cross-validate)")
        return

    total_tests = total_passed + total_failed

    # Ensure total from artifacts == CANONICAL_TOTAL
    if total_tests != CANONICAL_TOTAL:
        fail(
            f"playwright artifacts total ({total_tests}) "
            f"!= canonical total ({CANONICAL_TOTAL}); "
            f"shards present: {shards_present}"
        )

    # Ensure passed count matches PROGRESS.md completed
    if total_passed != completed:
        fail(
            f"playwright artifacts passed ({total_passed}) "
            f"!= PROGRESS.md completed ({completed}); "
            f"update PROGRESS.md to match artifact counts"
        )

    print(
        f"playwright artifacts ok: {total_passed}/{total_tests} passed "
        f"({total_failed} failed), "
        f"shards: {', '.join(shards_present)}"
    )


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

    # Check 3: displayed percentage matches computed (within 1%)
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

    # Check 5: gate-3-report contradiction linter
    lint_report(REPORT_FILE)

    # Check 6: playwright artifact cross-validation (when artifacts exist)
    lint_playwright_artifacts(completed)


if __name__ == "__main__":
    main()
