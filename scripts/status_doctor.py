#!/usr/bin/env python3
"""status_doctor: gate script required before any PROGRESS update.

Runs 5 checks in order:
  1. Denominator check        — bar N/D must equal CANONICAL_TOTAL
  2. Phase semantics check    — phase field consistent with counts
  3. Contradiction check      — no test in both DONE and FAIL sets
  4. Artifact freshness check — acceptance-bundle.json < 24h old
  5. Shard determinism check  — all expected shards have artifacts

Exit 0 if all checks pass.  Exit 1 on first failure.
Exit 2 on missing/unreadable required input.

Usage:
  python3 scripts/status_doctor.py
  python3 scripts/status_doctor.py --strict   # all checks, no skips
"""

import json
import re
import sys
import time
from pathlib import Path

ROOT = Path(__file__).parent.parent
PROGRESS_FILE = ROOT / "PROGRESS.md"
TMP = ROOT / "rsx-playground" / "tmp"
BUNDLE_PATH = TMP / "acceptance-bundle.json"
GATE3_REPORT = TMP / "gate-3-report.json"
PLAY_ARTIFACT_DIR = TMP / "play-artifacts"

CANONICAL_TOTAL = 340
BUNDLE_STALE_SECONDS = 86400

EXPECTED_SHARDS = [
    "routing",
    "htmx-partials",
    "process-control",
    "trade-ui",
]


# ── Utilities ─────────────────────────────────────────────────────────

def fail(msg: str, code: int = 1) -> None:
    print(f"[status-doctor] FAIL: {msg}", file=sys.stderr)
    sys.exit(code)


def ok(msg: str) -> None:
    print(f"[status-doctor] ok: {msg}")


# ── Check 1: Denominator ──────────────────────────────────────────────

def check_denominator() -> None:
    """Bar N/D denominator in PROGRESS.md must equal CANONICAL_TOTAL."""
    if not PROGRESS_FILE.exists():
        fail(f"PROGRESS.md not found: {PROGRESS_FILE}", code=2)

    text = PROGRESS_FILE.read_text()
    bars = [
        (int(m.group(1)), int(m.group(2)), int(m.group(3)))
        for m in re.finditer(
            r'\[[\u2588\u2591 ]+\]\s+(\d+)%\s+(\d+)/(\d+)', text
        )
    ]
    if not bars:
        fail("no progress bar found in PROGRESS.md")

    for pct, num, den in bars:
        if den != CANONICAL_TOTAL:
            fail(
                f"denominator mismatch: bar shows {num}/{den} "
                f"but canonical total is {CANONICAL_TOTAL}"
            )
        if num > den:
            fail(f"numerator {num} > denominator {den}")
        if den > 0:
            computed = round(100 * num / den)
            if abs(computed - pct) > 1:
                fail(
                    f"bar {num}/{den}: displayed {pct}% "
                    f"!= computed {computed}%"
                )

    last = bars[-1]
    ok(f"denominator={last[2]}, bar={last[1]}/{last[2]} ({last[0]}%)")


# ── Check 2: Phase semantics ──────────────────────────────────────────

def _parse_table_counts(text: str) -> dict[str, int]:
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


def check_phase_semantics() -> None:
    """Phase field must be semantically consistent with task counts."""
    if not PROGRESS_FILE.exists():
        fail(f"PROGRESS.md not found: {PROGRESS_FILE}", code=2)

    text = PROGRESS_FILE.read_text()
    m = re.search(r'^phase:\s*(\S+)', text, re.MULTILINE)
    phase = m.group(1) if m else "unknown"

    counts = _parse_table_counts(text)
    if len(counts) < 4:
        fail(
            f"PROGRESS.md table missing rows; found: "
            f"{list(counts.keys())}"
        )

    completed = counts.get("completed", 0)
    running = counts.get("running", 0)
    pending = counts.get("pending", 0)
    failed = counts.get("failed", 0)
    total = completed + running + pending + failed
    runnable = running + pending

    if phase == "complete":
        if completed != total:
            fail(
                f"phase=complete but completed ({completed}) "
                f"!= total ({total})"
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
                f"failed={failed} — stuck/zombie state"
            )
        if (
            completed == total
            and runnable == 0
            and failed == 0
            and total > 0
        ):
            fail(
                f"phase=executing but all {total} tasks complete "
                f"with nothing remaining; update phase to 'complete'"
            )
    else:
        print(
            f"[status-doctor] warn: unknown phase '{phase}' "
            f"(expected 'executing' or 'complete') — skipping",
            file=sys.stderr,
        )
        return

    ok(
        f"phase={phase}, completed={completed}/{total}, "
        f"running={running}, pending={pending}, failed={failed}"
    )


# ── Check 3: Contradiction ────────────────────────────────────────────

def check_contradiction() -> None:
    """No test may appear in both DONE and FAIL sets in gate-3-report."""
    if not GATE3_REPORT.exists():
        ok("gate-3-report.json absent — skipping contradiction check")
        return

    try:
        data = json.loads(GATE3_REPORT.read_text())
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

    ok(
        f"gate-3-report: {len(failed_tests)} failed, "
        f"no contradictions"
    )


# ── Check 4: Artifact freshness ───────────────────────────────────────

def check_artifact_freshness() -> None:
    """acceptance-bundle.json must exist and be < 24h old."""
    if not BUNDLE_PATH.exists():
        fail(
            "acceptance-bundle.json missing; "
            "run: make acceptance-bundle",
            code=2,
        )

    age = time.time() - BUNDLE_PATH.stat().st_mtime
    if age > BUNDLE_STALE_SECONDS:
        age_h = age / 3600
        fail(
            f"acceptance-bundle.json is stale "
            f"({age_h:.1f}h old, limit="
            f"{BUNDLE_STALE_SECONDS // 3600}h); "
            f"run: make acceptance-bundle"
        )

    age_m = age / 60
    ok(f"acceptance-bundle.json age={age_m:.1f}m (< 24h)")


# ── Check 5: Shard determinism ────────────────────────────────────────

def check_shard_determinism() -> None:
    """All expected playwright shards must have a report.json artifact.

    If the full-run artifact exists it satisfies the requirement alone.
    Otherwise each expected shard directory must have report.json.
    """
    full_run = PLAY_ARTIFACT_DIR / "full-run" / "report.json"
    if full_run.exists():
        ok("shard determinism: full-run/report.json present")
        return

    missing: list[str] = []
    present: list[str] = []
    for shard in EXPECTED_SHARDS:
        report = PLAY_ARTIFACT_DIR / shard / "report.json"
        if report.exists():
            present.append(shard)
        else:
            missing.append(shard)

    if missing:
        fail(
            f"shard artifacts missing: {', '.join(missing)}; "
            f"present: {present or 'none'}; "
            f"run the missing shards or make gate-4-playwright"
        )

    ok(f"shard determinism: all {len(present)} shards present")


# ── Main ──────────────────────────────────────────────────────────────

def main() -> None:
    print("[status-doctor] running 5 checks...")

    check_denominator()
    check_phase_semantics()
    check_contradiction()
    check_artifact_freshness()
    check_shard_determinism()

    print("[status-doctor] all checks passed")
    sys.exit(0)


if __name__ == "__main__":
    main()
