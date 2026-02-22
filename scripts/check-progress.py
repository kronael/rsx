#!/usr/bin/env python3
"""Validate PROGRESS.md is consistent with acceptance artifacts only.

Checks:
1. PROGRESS.md exists and is parseable
2. Progress bar is internally self-consistent (pct matches N/D)
3. gate-3-report.json: no test key in both DONE and FAIL sets
4. Playwright artifact cross-validation (when artifacts exist)
5. Phase semantics: reject zombie/stuck states
6. CI diff check: PROGRESS.md header matches artifact-derived output
   (delegates to publish-progress.py --check when artifacts present)

NOTE: Table counts are NOT cross-validated against tasks.json.
     PROGRESS.md is artifact-derived; tasks.json is informational only.

Exit 0 if consistent, 1 if inconsistent.
"""

import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).parent.parent
PROGRESS_FILE = ROOT / "PROGRESS.md"
TASKS_FILE = ROOT / ".ship" / "tasks.json"
REPORT_FILE = (
    ROOT / "rsx-playground" / "tmp" / "gate-3-report.json"
)
PLAY_ARTIFACT_DIR = (
    ROOT / "rsx-playground" / "tmp" / "play-artifacts"
)
PLAY_SHARDS = [
    "routing", "htmx-partials", "process-control", "trade-ui"
]


def fail(msg: str) -> None:
    print(f"PROGRESS inconsistency: {msg}", file=sys.stderr)
    sys.exit(1)


def parse_table_counts(text: str) -> dict[str, int]:
    """Parse the count-column counts from PROGRESS.md table."""
    counts: dict[str, int] = {}
    for label in ("completed", "running", "pending", "failed"):
        m = re.search(
            rf'\|\s*{label}\s*\|(?:[^\n|]*\|)*\s*(\d+)\s*\|',
            text,
            re.IGNORECASE,
        )
        if m:
            counts[label] = int(m.group(1))
    return counts


def parse_bars(text: str) -> list[tuple[int, int, int]]:
    """Return list of (pct, numerator, denominator) from all bars."""
    return [
        (int(m.group(1)), int(m.group(2)), int(m.group(3)))
        for m in re.finditer(
            r'\[[\u2588\u2591 ]+\]\s+(\d+)%\s+(\d+)/(\d+)', text
        )
    ]


def parse_phase(text: str) -> str:
    m = re.search(r'^phase:\s*(\S+)', text, re.MULTILINE)
    return m.group(1) if m else "unknown"


def check_bar_consistency(bars: list[tuple[int, int, int]]) -> None:
    """Each bar must be internally consistent: pct matches N/D."""
    for pct, num, den in bars:
        if den == 0:
            continue
        computed = round(100 * num / den)
        if abs(computed - pct) > 1:
            fail(
                f"bar {num}/{den}: displayed {pct}% != "
                f"computed {computed}%"
            )
        if num > den:
            fail(f"bar numerator {num} > denominator {den}")


def lint_report(path: Path) -> None:
    """Contradiction linter for gate-3-report.json snapshots."""
    if not path.exists():
        return
    try:
        data = json.loads(path.read_text())
    except Exception as e:
        fail(f"gate-3-report.json parse error: {e}")

    by_class = data.get("by_class", {})
    failed_tests: dict[str, str] = {}
    for ec, info in by_class.items():
        for entry in info.get("failures", []):
            tid = entry.get("test", "")
            if tid in failed_tests:
                fail(
                    f"contradiction: test in FAIL set of two classes: "
                    f"'{tid}' ('{failed_tests[tid]}' and '{ec}')"
                )
            failed_tests[tid] = ec

    full_results = data.get("results", [])
    if full_results:
        seen: dict[str, str] = {}
        for entry in full_results:
            tid = entry.get("test", "")
            outcome = entry.get("outcome", "")
            if tid in seen and seen[tid] != outcome:
                if tid in failed_tests:
                    fail(
                        f"contradiction: '{tid}' is in DONE set "
                        f"(outcome=passed) AND FAIL set "
                        f"(class='{failed_tests[tid]}')"
                    )
            seen[tid] = outcome
        for tid, ec in failed_tests.items():
            if seen.get(tid) == "passed":
                fail(
                    f"contradiction: '{tid}' appears as passed in "
                    f"results[] AND failed in by_class['{ec}']"
                )

    print(
        f"gate-3-report ok: {len(failed_tests)} failed, "
        f"no contradictions"
    )


def lint_playwright_artifacts() -> list[str]:
    """Cross-validate shard artifacts when present.

    Returns list of shards present (empty = not yet run).
    """
    total_passed = 0
    total_tests = 0
    shards_present: list[str] = []

    for shard in PLAY_SHARDS:
        report_file = PLAY_ARTIFACT_DIR / shard / "report.json"
        if not report_file.exists():
            continue
        try:
            data = json.loads(report_file.read_text())
        except Exception as e:
            fail(f"play-artifacts/{shard}/report.json: {e}")
        stats = data.get("stats", {})
        total_passed += stats.get("expected", 0)
        total_tests += stats.get("expected", 0) + stats.get(
            "unexpected", 0
        )
        shards_present.append(shard)

    if not shards_present:
        print("playwright artifacts: not yet run (skip cross-validate)")
        return []

    print(
        f"playwright artifacts ok: {total_passed}/{total_tests} "
        f"passed, shards: {', '.join(shards_present)}"
    )
    return shards_present


def check_phase_semantics(
    phase: str,
    counts: dict[str, int],
) -> None:
    """Phase field must be semantically consistent with counts."""
    completed = counts.get("completed", 0)
    running = counts.get("running", 0)
    pending = counts.get("pending", 0)
    failed = counts.get("failed", 0)
    total = completed + running + pending + failed
    runnable = running + pending

    if phase == "complete":
        if completed != total:
            fail(
                f"phase=complete but completed ({completed}) != "
                f"total ({total})"
            )
        if running != 0 or pending != 0:
            fail(
                f"phase=complete but running={running} "
                f"pending={pending}; requires zero runnable work"
            )
        if failed != 0:
            fail(
                f"phase=complete but failed={failed}; "
                f"complete phase requires zero failed tasks"
            )
    elif phase == "executing":
        if (
            runnable == 0
            and failed > 0
            and completed < total
        ):
            fail(
                f"phase=executing but runnable backlog is zero "
                f"(running={running}, pending={pending}) with "
                f"failed={failed} — stuck/zombie state; "
                f"requeue failed tasks or mark phase blocked"
            )
        if completed == total and runnable == 0 and failed == 0:
            fail(
                f"phase=executing but completed={completed}/{total} "
                f"with no remaining work; update phase to 'complete'"
            )
    else:
        print(
            f"phase semantics: unknown phase '{phase}' "
            f"(expected 'executing' or 'complete') — skipping",
            file=sys.stderr,
        )
        return

    print(
        f"phase semantics ok: phase={phase}, "
        f"completed={completed}/{total}, "
        f"running={running}, pending={pending}, failed={failed}"
    )


def ci_diff_check(shards_present: list[str]) -> None:
    """CI diff: verify PROGRESS.md header matches artifact recomputation.

    Delegates to publish-progress.py --check.  Only runs when both the
    acceptance bundle AND playwright shard artifacts are present (i.e.
    a full artifact set exists to recompute from).

    Exits 1 if divergence is detected.
    """
    bundle = ROOT / "rsx-playground" / "tmp" / "acceptance-bundle.json"
    if not bundle.exists():
        print(
            "ci-diff: acceptance-bundle.json absent"
            " — skipping recomputation check"
        )
        return
    if not shards_present:
        print(
            "ci-diff: no playwright shard artifacts"
            " — skipping recomputation check"
        )
        return

    result = subprocess.run(
        [sys.executable, str(ROOT / "scripts" / "publish-progress.py"),
         "--check"],
        capture_output=True,
        text=True,
    )
    if result.stdout:
        print(result.stdout.rstrip())
    if result.returncode == 0:
        print("ci-diff ok: PROGRESS.md header matches artifacts")
        return

    # Divergence or blocked
    if result.returncode == 2:
        # Missing artifact — already printed above, non-fatal here
        print(
            "ci-diff: publish-progress blocked (missing artifact)"
            " — skipping",
            file=sys.stderr,
        )
        return

    print(result.stderr.rstrip(), file=sys.stderr)
    fail(
        "PROGRESS.md header diverges from artifact-derived values"
        " (run: make publish-progress)"
    )


def main() -> None:
    # 1. Load PROGRESS.md.
    if not PROGRESS_FILE.exists():
        fail(f"PROGRESS.md not found: {PROGRESS_FILE}")
    text = PROGRESS_FILE.read_text()

    # 2. Parse table counts from PROGRESS.md.
    counts = parse_table_counts(text)
    if len(counts) < 4:
        fail(
            f"PROGRESS.md table missing rows; found: "
            f"{list(counts.keys())}"
        )

    # 3. Parse and validate progress bars (internal self-consistency only).
    bars = parse_bars(text)
    if not bars:
        fail("no progress bar found in PROGRESS.md")
    check_bar_consistency(bars)

    # 4. gate-3-report contradiction linter.
    lint_report(REPORT_FILE)

    # 5. Playwright artifact cross-validation (optional).
    shards_present = lint_playwright_artifacts()

    # 6. Phase semantics (derived from PROGRESS.md table, not tasks.json).
    phase = parse_phase(text)
    check_phase_semantics(phase, counts)

    # 7. CI diff: PROGRESS.md header must match artifact recomputation.
    ci_diff_check(shards_present)

    print(
        f"PROGRESS ok: bar={bars[-1][1]}/{bars[-1][2]}"
        f", phase={phase}"
    )


if __name__ == "__main__":
    main()
