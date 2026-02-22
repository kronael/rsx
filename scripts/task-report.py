#!/usr/bin/env python3
"""Truth-source reporter: update PROGRESS.md workers/log from tasks.json.

Reads task state from .ship/tasks.json and updates ONLY the workers and
log sections of PROGRESS.md.  The header block is preserved as-is (it is
written by publish-progress.py from acceptance-bundle.json, the sole
release-truth artifact).

Usage:
  python3 scripts/task-report.py             # update workers+log in PROGRESS.md
  python3 scripts/task-report.py --check     # check only, exit 1 on drift
  python3 scripts/task-report.py --print     # print counts, no file writes

Exit codes:
  0  ok (updated or counts match)
  1  drift detected (--check mode)
  2  tasks.json missing or unreadable
"""

import json
import re
import sys
from datetime import datetime
from pathlib import Path

# Status priority for collapse: lower = more authoritative.
_STATUS_PRIORITY = {"completed": 0, "running": 1, "pending": 2, "failed": 3}

ROOT = Path(__file__).parent.parent
TASKS_FILE = ROOT / ".ship" / "tasks.json"
PROGRESS = ROOT / "PROGRESS.md"

# Acceptance target: number of Playwright tests that must pass.
ACCEPTANCE_TARGET = 223

# Keywords that mark a task as release-critical (blocks 223 passing).
_RELEASE_CRITICAL = re.compile(
    r"playwright|fix 500|fix /x/|fix /api/|fix order|fix book"
    r"|fix risk|fix wal|fix maker|fix gateway|fix stress"
    r"|page routes|htmx partial|test all|rest/ws contract"
    r"|e2e test|acceptance|223",
    re.IGNORECASE,
)

# Patterns indicating unverifiable claims — stripped from generated log.
_UNSUPPORTED = re.compile(
    r"already (complete|implemented|in place|exist|ship|enforced)"
    r"|verified .{0,40} already"
    r"|\d+ tests? pass"
    r"|normalized test count"
    r"|audited .{0,30} already",
    re.IGNORECASE,
)


def task_signature(desc: str) -> str:
    """Return a stable dedup key for a task description.

    Strips leading verbs that vary across retries (Run, Fix, Verify,
    Execute, Re-run, Retry) and lowercases/collapses whitespace.
    Truncates to 60 chars so near-identical descriptions merge.
    """
    s = desc.strip().lower()
    s = re.sub(r'\s+', ' ', s)
    # Drop common retry-verb prefixes.
    s = re.sub(
        r'^(re-?run|retry|re-?execute|re-?verify|execute|run|fix|verify'
        r'|validate|check|add|update|implement|test|ensure)\s+',
        '',
        s,
    )
    return s[:60]


def collapse_tasks(tasks: list[dict]) -> list[dict]:
    """Collapse duplicate/retried tasks by signature.

    For each group of tasks with the same signature keep only the single
    most-authoritative task:
      completed (latest) > running (latest) > pending (latest)
                         > failed (latest)

    Superseded tasks (older attempts at the same work) are dropped from
    counts and from the log so they do not inflate pending/failed totals.
    """
    groups: dict[str, list[dict]] = {}
    for t in tasks:
        sig = task_signature(t.get("description", ""))
        groups.setdefault(sig, []).append(t)

    result: list[dict] = []
    for group in groups.values():
        if len(group) == 1:
            result.append(group[0])
            continue

        # Sort within each status bucket by timestamp descending.
        def _ts(t: dict) -> str:
            return (
                t.get("completed_at")
                or t.get("started_at")
                or t.get("created_at")
                or ""
            )

        group.sort(
            key=lambda t: (
                _STATUS_PRIORITY.get(t.get("status", ""), 99),
                _ts(t),
            )
        )
        # Lowest priority index wins; within same priority, latest ts.
        # group is sorted ascending by priority then ascending by ts, so
        # within the best priority bucket the last element is the latest.
        best_priority = _STATUS_PRIORITY.get(group[0].get("status", ""), 99)
        candidates = [
            t for t in group
            if _STATUS_PRIORITY.get(t.get("status", ""), 99) == best_priority
        ]
        result.append(candidates[-1])  # latest within best bucket

    return result


def classify_task(t: dict) -> str:
    """Return 'release' if the task blocks the 223-test target, else 'feature'."""
    desc = t.get("description", "")
    if _RELEASE_CRITICAL.search(desc):
        return "release"
    return "feature"


def split_tasks(tasks: list[dict]) -> tuple[list[dict], list[dict]]:
    """Return (release_critical, feature) task lists."""
    release, feature = [], []
    for t in tasks:
        (release if classify_task(t) == "release" else feature).append(t)
    return release, feature


def load_tasks() -> list[dict]:
    """Load and collapse tasks from tasks.json.

    Returns the collapsed task list (one task per signature) so that
    superseded retries and duplicate failed attempts are hidden from
    headline counts and the log section.
    """
    if not TASKS_FILE.exists():
        print(
            f"[task-report] ERROR: {TASKS_FILE} not found",
            file=sys.stderr,
        )
        sys.exit(2)
    try:
        data = json.loads(TASKS_FILE.read_text())
        raw = data if isinstance(data, list) else []
    except Exception as e:
        print(f"[task-report] ERROR: tasks.json: {e}", file=sys.stderr)
        sys.exit(2)
    return collapse_tasks(raw)


def count_tasks(tasks: list[dict]) -> dict[str, int]:
    counts: dict[str, int] = {
        "completed": 0,
        "running": 0,
        "pending": 0,
        "failed": 0,
    }
    for t in tasks:
        status = t.get("status", "")
        if status in counts:
            counts[status] += 1
        elif status:
            counts.setdefault(status, 0)
            counts[status] += 1
    return counts


def build_bar(completed: int, total: int) -> str:
    if total == 0:
        return "[░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░]   0%    0/0"
    pct = round(100 * completed / total)
    width = 30
    filled = round(width * completed / total)
    bar = "█" * filled + "░" * (width - filled)
    return f"[{bar}] {pct:3d}%  {completed}/{total}"


def build_header(
    counts: dict[str, int],
    release_counts: dict[str, int],
    feature_counts: dict[str, int],
) -> str:
    completed = counts["completed"]
    running = counts["running"]
    pending = counts["pending"]
    failed = counts["failed"]
    total = completed + running + pending + failed

    rc = release_counts["completed"]
    rt = sum(release_counts.values())
    fc = feature_counts["completed"]
    ft = sum(feature_counts.values())

    now = datetime.now().strftime("%b %d %H:%M:%S")
    # Primary bar: release-critical tasks vs acceptance target.
    bar = build_bar(rc, ACCEPTANCE_TARGET)
    phase = "complete" if (
        completed == total and running == 0 and pending == 0
    ) else "executing"

    return (
        f"# PROGRESS\n\n"
        f"updated: {now}  \n"
        f"phase: {phase}\n\n"
        f"```\n"
        f"acceptance  {bar}\n"
        f"tasks       {build_bar(completed, total)}\n"
        f"```\n\n"
        f"| | release-critical | feature | total |\n"
        f"|---|---|---|---|\n"
        f"| completed | {rc} | {fc} | {completed} |\n"
        f"| running   | {release_counts['running']}"
        f" | {feature_counts['running']} | {running} |\n"
        f"| pending   | {release_counts['pending']}"
        f" | {feature_counts['pending']} | {pending} |\n"
        f"| failed    | {release_counts['failed']}"
        f" | {feature_counts['failed']} | {failed} |\n"
        f"| **total** | **{rt}** | **{ft}** | **{total}** |"
    )


def build_workers(tasks: list[dict]) -> str:
    running = [t for t in tasks if t.get("status") == "running"]
    if not running:
        return ""
    lines = ["## workers", ""]
    for i, t in enumerate(running):
        desc = t.get("description", "").strip()
        lines.append(f"- w{i}: {desc}")
    return "\n".join(lines)


def build_log(tasks: list[dict]) -> str:
    """Build log section from task summaries — no manual entries."""
    entries = []
    for t in tasks:
        if t.get("status") != "completed":
            continue
        summary = (t.get("summary") or "").strip()
        if not summary:
            continue
        # Drop entries with unverifiable readiness claims.
        if _UNSUPPORTED.search(summary):
            continue
        ts_raw = t.get("completed_at") or ""
        # Extract HH:MM:SS from ISO timestamp.
        ts = ts_raw[11:19] if len(ts_raw) >= 19 else "??:??:??"
        entries.append((ts_raw, ts, summary))

    if not entries:
        return ""

    entries.sort(key=lambda x: x[0])
    lines = ["## log", ""]
    for _, ts, summary in entries:
        lines.append(f"- `{ts}` {summary}")
    return "\n".join(lines)


def extract_header(text: str) -> str:
    """Extract header block (everything before ## workers or ## log)."""
    m = re.search(r'^(##\s+workers|##\s+log)', text, re.MULTILINE)
    if m:
        return text[:m.start()].rstrip()
    return text.rstrip()


def build_progress(tasks: list[dict]) -> str:
    """Build PROGRESS.md from collapsed tasks.

    Rebuilds the header, workers, and log sections so all counts
    reflect the collapsed (deduplicated) task list.
    """
    counts = count_tasks(tasks)
    release_tasks, feature_tasks = split_tasks(tasks)
    release_counts = count_tasks(release_tasks)
    feature_counts = count_tasks(feature_tasks)

    header = build_header(counts, release_counts, feature_counts)
    workers = build_workers(tasks)
    log = build_log(tasks)

    parts = [header]
    if workers:
        parts.append(workers)
    if log:
        parts.append(log)

    return "\n\n".join(parts) + "\n"


def parse_counts(text: str) -> dict[str, int]:
    counts: dict[str, int] = {}
    for label in ("completed", "running", "pending", "failed"):
        # Match last numeric column on the row (total column).
        m = re.search(
            rf'\|\s*{label}\s*\|(?:[^\n|]*\|)*\s*(\d+)\s*\|',
            text,
            re.IGNORECASE,
        )
        if m:
            counts[label] = int(m.group(1))
    return counts


def main() -> None:
    check_only = "--check" in sys.argv
    print_only = "--print" in sys.argv

    tasks = load_tasks()
    counts = count_tasks(tasks)
    total = sum(counts.values())

    if print_only:
        print(
            f"task-report: "
            f"completed={counts['completed']} "
            f"running={counts['running']} "
            f"pending={counts['pending']} "
            f"failed={counts['failed']} "
            f"total={total}"
        )
        sys.exit(0)

    if check_only:
        if not PROGRESS.exists():
            print(
                "[task-report] MISSING: PROGRESS.md not found",
                file=sys.stderr,
            )
            sys.exit(1)
        existing = PROGRESS.read_text()
        existing_counts = parse_counts(existing)
        drift = False
        for label in ("completed", "running", "pending", "failed"):
            ev = existing_counts.get(label)
            gv = counts.get(label, 0)
            if ev != gv:
                if not drift:
                    print(
                        "[task-report] DRIFT: PROGRESS.md counts differ "
                        "from tasks.json",
                        file=sys.stderr,
                    )
                    drift = True
                print(
                    f"  {label}: PROGRESS.md={ev} tasks.json={gv}",
                    file=sys.stderr,
                )
        if drift:
            sys.exit(1)
        print(
            f"[task-report] ok: PROGRESS.md matches tasks.json "
            f"({counts['completed']}/{total})"
        )
        sys.exit(0)

    new_text = build_progress(tasks)
    PROGRESS.write_text(new_text)
    print(
        f"[task-report] written workers+log: "
        f"completed={counts['completed']} "
        f"running={counts['running']} "
        f"pending={counts['pending']} "
        f"failed={counts['failed']} "
        f"total={total}"
    )
    sys.exit(0)


if __name__ == "__main__":
    main()
