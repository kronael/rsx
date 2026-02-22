#!/usr/bin/env python3
"""Contradiction linter for .ship/tasks.json snapshots.

Rejects snapshot updates where any task id appears in both the DONE
set and the FAIL/retry set in the same update. Also validates the
snapshot file itself for internal contradictions.

Usage:
  # Lint the current snapshot for internal contradictions
  python3 scripts/lint-snapshot.py

  # Also cross-check PROGRESS.md rendered counts against tasks.json
  python3 scripts/lint-snapshot.py --progress-file PROGRESS.md

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
import re
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

    # Intra-record contradiction in the final rendered snapshot:
    # a task marked completed but retaining a non-empty error field is
    # stale done+fail duality — the record was completed over a failed
    # state without clearing the error field.
    for task in tasks:
        tid = task.get("id", "")
        if not tid:
            continue
        status = task.get("status", "")
        error = task.get("error", "")
        if status in DONE_STATUSES and error:
            errors.append(
                f"stale contradiction: task {tid!r} is completed "
                f"but has non-empty error field: {error!r}"
            )

    # Supersession edge cases: a task with a superseded_by reference
    # must not be active (running/pending). If it is, the snapshot
    # still shows it as live even though another task replaced it.
    for task in tasks:
        tid = task.get("id", "")
        if not tid:
            continue
        superseded_by = task.get("superseded_by", "")
        if not superseded_by:
            continue
        status = task.get("status", "")
        if status in {"running", "pending"}:
            errors.append(
                f"supersession conflict: task {tid!r} is {status!r} "
                f"but superseded_by={superseded_by!r}"
            )

    # Final rendered duality: a failed task whose retries field is
    # exhausted (retries == 0) but also has no error message is
    # an incomplete failure record — the rendered snapshot is missing
    # the reason for failure.
    for task in tasks:
        tid = task.get("id", "")
        if not tid:
            continue
        status = task.get("status", "")
        if status not in FAIL_STATUSES:
            continue
        retries = task.get("retries", None)
        error = task.get("error", "")
        if retries == 0 and not error:
            errors.append(
                f"incomplete failure: task {tid!r} is failed with "
                f"retries=0 but has no error message"
            )

    return errors


def parse_progress_counts(text: str) -> dict[str, int] | None:
    """Extract the status count table from a PROGRESS.md string.

    Looks for a markdown table block with rows like:
      | completed | 269 |
      | running   |   7 |
      | pending   |  64 |
      | failed    |   0 |

    Returns a dict mapping status -> count, or None if not found.
    """
    counts: dict[str, int] = {}
    for line in text.splitlines():
        m = re.match(
            r"^\|\s*(completed|running|pending|failed)\s*\|\s*(\d+)\s*\|",
            line.strip(),
        )
        if m:
            counts[m.group(1)] = int(m.group(2))
    if not counts:
        return None
    return counts


def lint_progress_md(
    tasks: list[dict], progress_text: str, progress_path: str = "PROGRESS.md"
) -> list[str]:
    """Cross-check the rendered PROGRESS.md counts against tasks.json.

    The PROGRESS.md table is the *final rendered snapshot* — it must
    match the actual task counts from tasks.json exactly. Any drift
    is a contradiction between the rendered artifact and the truth
    source.
    """
    errors: list[str] = []

    rendered = parse_progress_counts(progress_text)
    if rendered is None:
        errors.append(
            f"progress file {progress_path!r} contains no status count table"
        )
        return errors

    actual: dict[str, int] = {
        "completed": 0,
        "running": 0,
        "pending": 0,
        "failed": 0,
    }
    for task in tasks:
        status = task.get("status", "")
        if status in actual:
            actual[status] += 1

    for status in ("completed", "running", "pending", "failed"):
        rendered_count = rendered.get(status, 0)
        actual_count = actual[status]
        if rendered_count != actual_count:
            errors.append(
                f"progress drift: {status} count in {progress_path!r} is "
                f"{rendered_count} but tasks.json has {actual_count}"
            )

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
    progress_file: str | None = None

    i = 0
    while i < len(args):
        if args[i] == "--update" and i + 1 < len(args):
            update_json = args[i + 1]
            i += 2
        elif args[i] == "--update-file" and i + 1 < len(args):
            update_file = args[i + 1]
            i += 2
        elif args[i] == "--progress-file" and i + 1 < len(args):
            progress_file = args[i + 1]
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

    if progress_file is not None:
        try:
            progress_text = Path(progress_file).read_text()
        except OSError as e:
            print(
                f"[lint-snapshot] ERROR: progress file error: {e}",
                file=sys.stderr,
            )
            sys.exit(2)
        errors.extend(lint_progress_md(tasks, progress_text, progress_file))

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
