#!/usr/bin/env python3
"""Deterministic exit criteria for .ship/tasks.json.

A task may only stay "completed" when ALL of the following hold:
  1. ALL linked failing_test_ids are green (not in bundle failing_ids)
  2. The bundle commit_sha matches current HEAD
  3. The Playwright full-run artifact predates the bundle generated_at
     (i.e. the bundle summarises the artifact, not the reverse)

Otherwise the task is auto-reopened (status → "pending", error set).

Fields read from each task:
  failing_test_ids: list[str]  — test IDs that must be green
                                 (absent or [] = no test-gate, task keeps
                                 completed status)
  status: str                  — "completed" | "pending" | "running" | "failed"

Sources:
  .ship/tasks.json                         — task state
  rsx-playground/tmp/acceptance-bundle.json — current test results + HEAD SHA

Exit codes:
  0  all completed tasks satisfy exit criteria (or have no linked IDs)
  1  one or more tasks were reopened
  2  acceptance bundle missing or unreadable
  3  dry-run: would have reopened N task(s) (printed, not written)

Usage:
  python3 scripts/exit-criteria.py             # check + auto-reopen
  python3 scripts/exit-criteria.py --dry-run   # report only, no writes
  python3 scripts/exit-criteria.py --verbose   # show per-task decisions
"""

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).parent.parent
TASKS_FILE = ROOT / ".ship" / "tasks.json"
BUNDLE_PATH = ROOT / "rsx-playground" / "tmp" / "acceptance-bundle.json"


def git_sha() -> str:
    try:
        return subprocess.check_output(
            ["git", "rev-parse", "--short", "HEAD"],
            cwd=ROOT,
            stderr=subprocess.DEVNULL,
        ).decode().strip()
    except Exception:
        return "unknown"


def load_bundle() -> dict:
    if not BUNDLE_PATH.exists():
        print(
            "[exit-criteria] ERROR: acceptance bundle missing: "
            f"{BUNDLE_PATH}\n"
            "  Run: python3 scripts/acceptance-bundle.py",
            file=sys.stderr,
        )
        sys.exit(2)
    try:
        return json.loads(BUNDLE_PATH.read_text())
    except Exception as e:
        print(f"[exit-criteria] ERROR: bundle parse error: {e}", file=sys.stderr)
        sys.exit(2)


def load_tasks() -> list[dict]:
    if not TASKS_FILE.exists():
        return []
    try:
        data = json.loads(TASKS_FILE.read_text())
        return data if isinstance(data, list) else []
    except Exception as e:
        print(f"[exit-criteria] ERROR: tasks.json parse error: {e}", file=sys.stderr)
        sys.exit(2)


def passing_ids_from_bundle(bundle: dict) -> set[str]:
    """Collect all test IDs that are currently passing.

    The acceptance bundle records failing_ids only.  We derive passing IDs
    from the Playwright shard artifacts (walking suites) and the gate-3
    API report (full_results list).

    For a lighter approach we invert: a test ID is passing iff it does NOT
    appear in bundle["failing_ids"].

    For API tests (gate-3 by_class), IDs in failing list are failing; the
    rest from that class are passing.  We represent API failing IDs as:
      "[api/<class>] <test_name>"
    matching the format written by all_failing_ids() in acceptance-bundle.py.
    """
    # All IDs that appear as failing in the bundle
    failing: set[str] = set(bundle.get("failing_ids", []))
    return failing  # caller uses: test_id NOT in failing_set → passing


def check_bundle_head(bundle: dict, current_sha: str) -> bool:
    """Return True if bundle commit SHA matches current HEAD."""
    bundle_sha = bundle.get("commit_sha", "")
    return bool(bundle_sha) and bundle_sha == current_sha


def check_artifact_timestamp(bundle: dict) -> bool:
    """Return True if the Playwright full-run artifact predates the bundle.

    Guards against accepting results where the full-run artifact was
    regenerated after the bundle was written (bundle would be stale
    relative to the artifact it claims to summarise).

    Passes when:
      - full-run/report.json does not exist (no artifact — gate4 will
        handle this as not-run; timestamp constraint doesn't apply)
      - artifact mtime <= bundle generated_at (artifact was present when
        bundle was generated, not modified since)
    """
    bundle_ts = bundle.get("generated_at", 0)
    if not bundle_ts:
        return False  # bundle missing timestamp — treat as stale

    artifact = BUNDLE_PATH.parent / "play-artifacts" / "full-run" / "report.json"
    if not artifact.exists():
        return True  # no artifact; gate4 handles absence separately

    try:
        artifact_mtime = artifact.stat().st_mtime
    except OSError:
        return True  # can't stat — skip check

    # Artifact must not post-date the bundle (allow 1s clock skew)
    return artifact_mtime <= bundle_ts + 1


def run(dry_run: bool = False, verbose: bool = False) -> int:
    """Main logic. Returns number of tasks reopened (or would-reopen in dry-run)."""
    bundle = load_bundle()
    tasks = load_tasks()
    current_sha = git_sha()

    # Verify HEAD matches bundle
    same_head = check_bundle_head(bundle, current_sha)
    if not same_head:
        bundle_sha = bundle.get("commit_sha", "unknown")
        print(
            f"[exit-criteria] WARN: bundle SHA ({bundle_sha}) != "
            f"HEAD ({current_sha}); all linked-ID checks will fail "
            f"(stale bundle — regenerate with acceptance-bundle.py)",
            file=sys.stderr,
        )

    # Verify artifact timestamp matches bundle generation time
    artifact_ok = check_artifact_timestamp(bundle)
    if not artifact_ok:
        print(
            "[exit-criteria] WARN: Playwright full-run artifact was modified "
            "after the bundle was generated; linked-ID checks will fail "
            "(regenerate bundle with acceptance-bundle.py)",
            file=sys.stderr,
        )

    failing_in_bundle: set[str] = passing_ids_from_bundle(bundle)

    reopened: list[str] = []

    for task in tasks:
        if task.get("status") != "completed":
            continue

        linked: list[str] = task.get("failing_test_ids", [])
        if not linked:
            # No test gate — keep completed
            if verbose:
                print(
                    f"[exit-criteria] KEEP   {task['id'][:8]}  "
                    f"(no linked test IDs)"
                )
            continue

        # Check each linked ID
        still_failing = [t for t in linked if t in failing_in_bundle]
        not_in_bundle = []

        if not same_head or not artifact_ok:
            # Bundle or artifact is stale — treat all linked IDs as
            # unverified = reopen
            still_failing = linked  # all unverified
        else:
            # Accept: ID not in failing_in_bundle means it passed.
            still_failing = [t for t in linked if t in failing_in_bundle]

        if still_failing:
            reopened.append(task["id"])
            if verbose or dry_run:
                prefix = "[DRY-RUN] " if dry_run else ""
                print(
                    f"[exit-criteria] {prefix}REOPEN {task['id'][:8]}  "
                    f"'{task.get('description', '')[:60]}'  "
                    f"— {len(still_failing)}/{len(linked)} linked "
                    f"test(s) still failing:"
                )
                for t in still_failing:
                    print(f"    ✗ {t}")
            elif not dry_run:
                print(
                    f"[exit-criteria] REOPEN {task['id'][:8]}  "
                    f"'{task.get('description', '')[:60]}'  "
                    f"({len(still_failing)} test(s) still failing)"
                )
            if not dry_run:
                task["status"] = "pending"
                task["error"] = (
                    f"auto-reopened: {len(still_failing)} linked test(s) "
                    f"still failing on HEAD {current_sha}: "
                    + ", ".join(still_failing[:3])
                    + ("..." if len(still_failing) > 3 else "")
                )
        else:
            if verbose:
                print(
                    f"[exit-criteria] KEEP   {task['id'][:8]}  "
                    f"'{task.get('description', '')[:60]}'  "
                    f"(all {len(linked)} linked test(s) green)"
                )

    if reopened and not dry_run:
        TASKS_FILE.write_text(json.dumps(tasks, indent=2))
        print(
            f"[exit-criteria] wrote tasks.json: "
            f"{len(reopened)} task(s) reopened"
        )

    return len(reopened)


def main() -> None:
    dry_run = "--dry-run" in sys.argv
    verbose = "--verbose" in sys.argv or "-v" in sys.argv

    n = run(dry_run=dry_run, verbose=verbose)

    if n == 0:
        print("[exit-criteria] ok: all completed tasks satisfy exit criteria")
        sys.exit(0)
    elif dry_run:
        print(
            f"[exit-criteria] dry-run: would reopen {n} task(s)",
            file=sys.stderr,
        )
        sys.exit(3)
    else:
        sys.exit(1)


if __name__ == "__main__":
    main()
