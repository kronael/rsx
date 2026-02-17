#!/usr/bin/env python3
"""Contradiction linter for .ship/tasks.json snapshots.

Rejects snapshot updates where any task id appears in both the DONE
set and the FAIL/retry set in the same update. Also validates the
snapshot file itself for internal contradictions.

Usage:
  # Lint the current snapshot for internal contradictions
  python3 scripts/lint-snapshot.py

  # Lint a proposed update before applying it
  python3 scripts/lint-snapshot.py --update '{"done":["id1"],"fail":["id1"]}'

  # Lint an update from a file
  python3 scripts/lint-snapshot.py --update-file path/to/update.json

Exit codes:
  0  no contradictions found
  1  contradictions found (printed to stderr)
  2  snapshot file missing or unparseable
"""

import json
import sys
from pathlib import Path

ROOT = Path(__file__).parent.parent
SNAPSHOT = ROOT / ".ship" / "tasks.json"

DONE_STATUSES = {"completed"}
FAIL_STATUSES = {"failed"}
RETRY_STATUSES = {"failed"}  # tasks that are failed and have retries > 0


def load_snapshot() -> list[dict]:
    if not SNAPSHOT.exists():
        print(f"[lint-snapshot] ERROR: {SNAPSHOT} not found", file=sys.stderr)
        sys.exit(2)
    try:
        data = json.loads(SNAPSHOT.read_text())
    except json.JSONDecodeError as e:
        print(f"[lint-snapshot] ERROR: parse error: {e}", file=sys.stderr)
        sys.exit(2)
    if not isinstance(data, list):
        print("[lint-snapshot] ERROR: tasks.json must be a JSON array", file=sys.stderr)
        sys.exit(2)
    return data


def lint_snapshot(tasks: list[dict]) -> list[str]:
    """Check the snapshot itself for internal contradictions.

    A snapshot is contradictory if the same task id appears more than
    once with conflicting statuses (one completed, one failed/running).
    """
    errors: list[str] = []
    seen: dict[str, str] = {}  # id -> status

    for task in tasks:
        tid = task.get("id", "")
        status = task.get("status", "")
        if not tid:
            errors.append("task missing id field")
            continue
        if tid in seen:
            prev = seen[tid]
            if prev != status:
                errors.append(
                    f"duplicate id {tid!r}: status {prev!r} and {status!r}"
                )
        else:
            seen[tid] = status

    # Check for tasks marked completed that also have retries pending
    # (retries > 0 AND completed = suspicious but not necessarily contradictory)
    # Real contradiction: a task in DONE_STATUSES that's also in FAIL_STATUSES
    # — impossible within a single snapshot unless duplicated (caught above).
    return errors


def lint_update(tasks: list[dict], update: dict) -> list[str]:
    """Check a proposed update for contradictions before applying it.

    An update is contradictory if any task id appears in both:
      - update["done"]: tasks to mark completed
      - update["fail"]: tasks to mark failed (possibly re-queued for retry)
    """
    errors: list[str] = []

    done_ids: set[str] = set(update.get("done", []))
    fail_ids: set[str] = set(update.get("fail", []))
    retry_ids: set[str] = set(update.get("retry", []))

    # done ∩ fail contradiction
    both_done_fail = done_ids & fail_ids
    for tid in sorted(both_done_fail):
        errors.append(
            f"contradiction: task {tid!r} in both done and fail sets"
        )

    # done ∩ retry contradiction
    both_done_retry = done_ids & retry_ids
    for tid in sorted(both_done_retry):
        errors.append(
            f"contradiction: task {tid!r} in both done and retry sets"
        )

    # fail ∩ already-completed in snapshot
    existing: dict[str, str] = {t.get("id", ""): t.get("status", "") for t in tasks}
    for tid in sorted(fail_ids | retry_ids):
        if existing.get(tid) in DONE_STATUSES:
            errors.append(
                f"contradiction: task {tid!r} already completed in snapshot "
                f"but update marks it failed/retry"
            )

    # done ∩ already-failed with no retries left (marking done a permanently-failed task)
    # This is informational — not necessarily a bug, so we warn rather than error.
    for tid in sorted(done_ids):
        snap_status = existing.get(tid)
        if snap_status in FAIL_STATUSES:
            errors.append(
                f"contradiction: task {tid!r} already failed in snapshot "
                f"but update marks it done"
            )

    return errors


def main() -> None:
    args = sys.argv[1:]
    update_json: str | None = None
    update_file: str | None = None

    i = 0
    while i < len(args):
        if args[i] == "--update" and i + 1 < len(args):
            update_json = args[i + 1]
            i += 2
        elif args[i] == "--update-file" and i + 1 < len(args):
            update_file = args[i + 1]
            i += 2
        else:
            i += 1

    tasks = load_snapshot()

    errors: list[str] = []

    # Always lint the snapshot itself
    errors.extend(lint_snapshot(tasks))

    # If an update is provided, lint it too
    if update_json is not None:
        try:
            update = json.loads(update_json)
        except json.JSONDecodeError as e:
            print(f"[lint-snapshot] ERROR: update parse error: {e}", file=sys.stderr)
            sys.exit(2)
        errors.extend(lint_update(tasks, update))

    if update_file is not None:
        try:
            update = json.loads(Path(update_file).read_text())
        except (json.JSONDecodeError, OSError) as e:
            print(f"[lint-snapshot] ERROR: update file error: {e}", file=sys.stderr)
            sys.exit(2)
        errors.extend(lint_update(tasks, update))

    if errors:
        print(f"[lint-snapshot] FAIL: {len(errors)} contradiction(s):", file=sys.stderr)
        for e in errors:
            print(f"  - {e}", file=sys.stderr)
        sys.exit(1)

    total = len(tasks)
    done = sum(1 for t in tasks if t.get("status") in DONE_STATUSES)
    failed = sum(1 for t in tasks if t.get("status") in FAIL_STATUSES)
    print(
        f"[lint-snapshot] ok — {total} tasks, "
        f"{done} completed, {failed} failed, no contradictions"
    )
    sys.exit(0)


if __name__ == "__main__":
    main()
