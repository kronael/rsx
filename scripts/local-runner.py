#!/usr/bin/env python3
"""Fallback local script runner for blocked external-agent sessions.

When Claude Code agent sessions are quota-blocked, tasks in .ship/tasks.json
can be stuck in 'running' state.  This script detects those blocked tasks
and executes the equivalent release-validation make targets directly,
without requiring an agent session.

Usage:
  python3 scripts/local-runner.py              # run all blocked tasks
  python3 scripts/local-runner.py --dry-run    # show what would run
  python3 scripts/local-runner.py --pending    # also pick up pending tasks
  python3 scripts/local-runner.py --id <id>    # run one task by id prefix
  python3 scripts/local-runner.py --auto       # auto-switch: run if stale
  python3 scripts/local-runner.py --stale-mins N  # stale threshold (default 30)
  python3 scripts/local-runner.py --head-only  # fresh HEAD-only pipeline cycle

--head-only mode:
  Executes one deterministic cycle against current HEAD:
    gate-1-startup → gate-2-partials → gate-3-api →
    gate-4-playwright → acceptance-bundle → gen-release-truth
  After the pipeline, verifies that every produced artifact:
    - has commit_sha matching current HEAD (SHA check)
    - has generated_at / mtime >= run start time (timestamp check)
  Exits 1 if any gate fails or any artifact timestamp/SHA differs.

Executor mode artifact:
  tmp/executor-mode.json  - written on every run, records executor=local|agent

Exit codes:
  0  all targeted tasks passed (or nothing to do in --auto with no stale tasks)
  1  one or more tasks failed
  2  tasks.json missing or unreadable
"""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import time
from datetime import datetime
from datetime import timezone
from pathlib import Path

ROOT = Path(__file__).parent.parent
TASKS_FILE = ROOT / ".ship" / "tasks.json"
ARTIFACT_FILE = ROOT / "tmp" / "executor-mode.json"

# Lazy-load meta-guard so local-runner works even if meta-guard.py
# is absent (older checkouts, partial deploys).
_meta_guard = None


def _load_meta_guard():
    global _meta_guard
    if _meta_guard is not None:
        return _meta_guard
    mg_path = Path(__file__).parent / "meta-guard.py"
    if not mg_path.exists():
        return None
    try:
        spec = importlib.util.spec_from_file_location(
            "meta_guard", mg_path,
        )
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        _meta_guard = mod
        return mod
    except Exception:
        return None


def meta_guard_blocked() -> bool:
    """Return True if meta-guard says new meta tasks are blocked.

    Calls meta-guard.run() without writing trend data (read-only
    check).  Returns False if meta-guard is unavailable or the
    bundle is missing (fail-open: don't block when we can't check).
    """
    mg = _load_meta_guard()
    if mg is None:
        return False
    try:
        code = mg.run(dry_run=True)
        # run() returns 3 for dry-run blocked, 1 for blocked.
        return code in (1, 3)
    except SystemExit as exc:
        # Missing bundle (exit 2) → fail-open.
        return exc.code not in (None, 0, 2)
    except Exception:
        return False

# Default stale threshold for --auto mode (minutes).
DEFAULT_STALE_MINS = 30

# Map task description keywords to make targets (ordered, first match wins).
# Each entry: (keywords_all_required, make_targets_in_order)
TASK_MAP: list[tuple[list[str], list[str]]] = [
    # Full validation suite
    (["gate", "all"], ["gate"]),
    (["ci", "full"], ["ci-full"]),
    (["ci"], ["ci"]),
    # Individual gates
    (["startup", "import", "healthz"], ["gate-1-startup"]),
    (["partial", "htmx", "route"], ["gate-2-partials"]),
    (["api", "test"], ["gate-3-api"]),
    (["playwright", "e2e"], ["gate-4-playwright"]),
    # Shard-level validation
    (["maker", "market"], ["shard-maker"]),
    (["trade", "ui"], ["shard-trade"]),
    (["routing", "navigation"], ["shard-routing"]),
    (["infra", "smoke"], ["shard-infra-smoke"]),
    # Build + unit tests
    (["build", "binar"], ["check", "test"]),
    (["cargo", "check"], ["check"]),
    (["rust", "unit"], ["test"]),
    # Acceptance bundle + progress
    (["acceptance", "bundle"], ["acceptance-bundle"]),
    (["progress", "accounting"], ["check-progress"]),
    # Generic "verify / test" fallback → run gates 1-3
    (["verify", "server"], ["gate-1-startup", "gate-2-partials"]),
    (["verify"], ["gate-1-startup", "gate-2-partials", "gate-3-api"]),
    (["test"], ["gate-1-startup", "gate-2-partials", "gate-3-api"]),
]


def record_executor_mode(
    mode: str,
    *,
    ran: int = 0,
    failed: int = 0,
    reason: str = "",
) -> None:
    """Write executor mode artifact to tmp/executor-mode.json."""
    ARTIFACT_FILE.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "executor": mode,
        "ts": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "ran": ran,
        "failed": failed,
        "reason": reason,
    }
    ARTIFACT_FILE.write_text(json.dumps(payload, indent=2))


def _parse_iso(ts: str) -> datetime | None:
    """Parse ISO-8601 timestamp to UTC-aware datetime, or None on error."""
    if not ts:
        return None
    try:
        # Replace Z suffix for Python <3.11 compatibility
        dt = datetime.fromisoformat(ts.replace("Z", "+00:00"))
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=timezone.utc)
        return dt
    except ValueError:
        return None


def detect_stale_running(
    tasks: list[dict],
    stale_mins: int = DEFAULT_STALE_MINS,
) -> list[dict]:
    """Return tasks in 'running' state whose started_at is older than
    stale_mins minutes.  These are presumed quota-blocked."""
    now = datetime.now(tz=timezone.utc)
    stale = []
    for task in tasks:
        if task.get("status") != "running":
            continue
        started = _parse_iso(task.get("started_at", ""))
        if started is None:
            # No timestamp → treat as stale (unknown age)
            stale.append(task)
            continue
        age_mins = (now - started).total_seconds() / 60
        if age_mins >= stale_mins:
            stale.append(task)
    return stale


def load_tasks() -> list[dict]:
    if not TASKS_FILE.exists():
        print(
            f"[local-runner] ERROR: {TASKS_FILE} not found",
            file=sys.stderr,
        )
        sys.exit(2)
    try:
        data = json.loads(TASKS_FILE.read_text())
        return data if isinstance(data, list) else []
    except Exception as e:
        print(f"[local-runner] ERROR: tasks.json parse error: {e}", file=sys.stderr)
        sys.exit(2)


def save_tasks(tasks: list[dict]) -> None:
    TASKS_FILE.write_text(json.dumps(tasks, indent=2))


def match_targets(description: str) -> list[str]:
    """Return make targets for a task description, or [] if no match."""
    desc = description.lower()
    for keywords, targets in TASK_MAP:
        if all(k in desc for k in keywords):
            return targets
    return []


def run_make(targets: list[str]) -> tuple[bool, str]:
    """Run make targets; return (success, combined output)."""
    cmd = ["make"] + targets
    try:
        result = subprocess.run(
            cmd,
            cwd=ROOT,
            capture_output=True,
            text=True,
            timeout=600,
        )
        output = result.stdout + result.stderr
        return result.returncode == 0, output
    except subprocess.TimeoutExpired:
        return False, "make timed out after 600s"
    except Exception as e:
        return False, str(e)


def run(
    dry_run: bool = False,
    include_pending: bool = False,
    target_id: str = "",
    auto: bool = False,
    stale_mins: int = DEFAULT_STALE_MINS,
) -> int:
    tasks = load_tasks()
    now = time.strftime("%Y-%m-%dT%H:%M:%S")
    failed = 0
    ran = 0

    # --auto: only run if stale blocked tasks exist; record mode + bail early.
    if auto and not dry_run:
        stale = detect_stale_running(tasks, stale_mins)
        if not stale:
            print(
                f"[local-runner] auto: no stale running tasks "
                f"(threshold={stale_mins}min) → executor=agent"
            )
            record_executor_mode("agent", reason="no stale tasks")
            return 0
        ids = ", ".join(t.get("id", "?")[:8] for t in stale)
        print(
            f"[local-runner] auto: {len(stale)} stale task(s) detected "
            f"→ switching executor=local  [{ids}]"
        )

    for task in tasks:
        task_id = task.get("id", "")
        status = task.get("status", "")
        description = task.get("description", "")

        # Filter by id prefix if given
        if target_id and not task_id.startswith(target_id):
            continue

        # Only handle blocked tasks (running with no active agent) or pending
        eligible = status == "running" or (include_pending and status == "pending")
        if not eligible and not target_id:
            continue
        # If target_id given, allow any non-completed task
        if target_id and status == "completed":
            print(f"[local-runner] SKIP  {task_id[:8]} already completed")
            continue

        targets = match_targets(description)
        if not targets:
            print(
                f"[local-runner] SKIP  {task_id[:8]} "
                f"no make target for: {description[:60]}"
            )
            continue

        # Meta-orchestration guard: skip meta tasks when blocked.
        mg = _load_meta_guard()
        if mg is not None and mg.is_meta_task(description):
            if meta_guard_blocked():
                print(
                    f"[local-runner] GUARD {task_id[:8]} "
                    f"meta task blocked by meta-guard: "
                    f"{description[:55]}"
                )
                continue

        targets_str = " ".join(targets)
        print(
            f"[local-runner] {'DRY ' if dry_run else ''}RUN  "
            f"{task_id[:8]}  make {targets_str}"
        )
        print(f"           desc: {description[:70]}")

        if dry_run:
            continue

        ran += 1
        ok, output = run_make(targets)

        # Truncate output for storage
        snippet = output[-2000:] if len(output) > 2000 else output

        if ok:
            task["status"] = "completed"
            task["result"] = (
                f"[local-runner] make {targets_str} passed at {now}\n\n"
                + snippet
            )
            task["summary"] = f"make {targets_str} passed (local runner)"
            task["error"] = ""
            print(f"[local-runner] PASS  {task_id[:8]}")
        else:
            task["status"] = "failed"
            task["error"] = f"make {targets_str} failed (local runner)"
            task["result"] = (
                f"[local-runner] make {targets_str} FAILED at {now}\n\n"
                + snippet
            )
            failed += 1
            print(f"[local-runner] FAIL  {task_id[:8]}")
            # Print last 20 lines of output for diagnosis
            lines = output.splitlines()[-20:]
            for line in lines:
                print(f"  {line}")

    if not dry_run and ran > 0:
        save_tasks(tasks)
        print(f"[local-runner] wrote tasks.json ({ran} tasks updated)")

    if ran == 0 and not dry_run:
        print(
            "[local-runner] no eligible tasks found"
            " (use --pending to include pending tasks)"
        )

    if not dry_run:
        mode = "local" if ran > 0 else "agent"
        reason = (
            f"ran {ran} task(s)"
            if ran > 0
            else "no eligible tasks; defaulting to agent"
        )
        record_executor_mode(mode, ran=ran, failed=failed, reason=reason)

    return failed


# Ordered pipeline for --head-only mode.
HEAD_ONLY_PIPELINE: list[str] = [
    "gate-1-startup",
    "gate-2-partials",
    "gate-3-api",
    "gate-4-playwright",
    "acceptance-bundle",
    "gen-release-truth",
]

PLAYGROUND_TMP = ROOT / "rsx-playground" / "tmp"


def _head_sha() -> str:
    """Read current HEAD SHA from .git without subprocess."""
    git_dir = ROOT / ".git"
    try:
        head = (git_dir / "HEAD").read_text().strip()
        if head.startswith("ref: "):
            ref = head[5:]
            ref_file = git_dir / ref
            if ref_file.exists():
                return ref_file.read_text().strip()[:7]
            packed = git_dir / "packed-refs"
            if packed.exists():
                for line in packed.read_text().splitlines():
                    if line.startswith("#"):
                        continue
                    parts = line.split()
                    if len(parts) == 2 and parts[1] == ref:
                        return parts[0][:7]
            return "unknown"
        return head[:7]
    except Exception:
        return "unknown"


def _verify_artifacts(run_start: float, head: str) -> list[str]:
    """Check that produced artifacts match HEAD SHA and this run's timestamp.

    Returns list of violation messages (empty = all ok).
    """
    issues: list[str] = []

    bundle_path = PLAYGROUND_TMP / "acceptance-bundle.json"
    truth_path = PLAYGROUND_TMP / "release_truth.json"
    full_run_path = (
        PLAYGROUND_TMP / "play-artifacts" / "full-run" / "report.json"
    )

    # --- acceptance-bundle.json ---
    if not bundle_path.exists():
        issues.append("acceptance-bundle.json missing after pipeline")
    else:
        try:
            bundle = json.loads(bundle_path.read_text())
            b_sha = bundle.get("commit_sha", "")
            b_ts = bundle.get("generated_at", 0)
            if b_sha and b_sha != head:
                issues.append(
                    f"acceptance-bundle SHA mismatch: "
                    f"artifact={b_sha} HEAD={head}"
                )
            if b_ts < run_start:
                issues.append(
                    f"acceptance-bundle timestamp predates run start: "
                    f"generated_at={b_ts} run_start={run_start:.0f}"
                )
        except Exception as exc:
            issues.append(f"acceptance-bundle.json parse error: {exc}")

    # --- release_truth.json ---
    if not truth_path.exists():
        issues.append("release_truth.json missing after pipeline")
    else:
        try:
            truth = json.loads(truth_path.read_text())
            t_sha = truth.get("commit_sha", "")
            t_ts = truth.get("generated_at", 0)
            if t_sha and t_sha != head:
                issues.append(
                    f"release_truth SHA mismatch: "
                    f"artifact={t_sha} HEAD={head}"
                )
            if t_ts < run_start:
                issues.append(
                    f"release_truth timestamp predates run start: "
                    f"generated_at={t_ts} run_start={run_start:.0f}"
                )
        except Exception as exc:
            issues.append(f"release_truth.json parse error: {exc}")

    # --- full-run/report.json ---
    if full_run_path.exists():
        try:
            mtime = full_run_path.stat().st_mtime
            if mtime < run_start:
                issues.append(
                    f"full-run/report.json predates run start: "
                    f"mtime={mtime:.0f} run_start={run_start:.0f}"
                )
        except OSError as exc:
            issues.append(f"full-run/report.json stat error: {exc}")

    return issues


def run_head_only() -> int:
    """Execute one fresh HEAD-only pipeline cycle.

    Runs the full pipeline sequentially, then verifies that every
    produced artifact has the current HEAD SHA and a timestamp >= the
    run start time.

    Returns 0 on success, 1 on any gate failure or artifact mismatch.
    """
    head = _head_sha()
    run_start = time.time()
    ts = time.strftime("%Y-%m-%dT%H:%M:%S")
    print(
        f"[local-runner] --head-only: HEAD={head} start={ts}"
    )

    failed_step: str = ""
    for target in HEAD_ONLY_PIPELINE:
        print(f"[local-runner] head-only: make {target}")
        ok, output = run_make([target])
        if not ok:
            failed_step = target
            lines = output.splitlines()[-20:]
            for line in lines:
                print(f"  {line}")
            print(
                f"[local-runner] FAIL head-only: make {target} failed"
            )
            break
        print(f"[local-runner] PASS head-only: make {target}")

    if failed_step:
        record_executor_mode(
            "local",
            ran=HEAD_ONLY_PIPELINE.index(failed_step) + 1,
            failed=1,
            reason=f"head-only: {failed_step} failed",
        )
        return 1

    # All pipeline steps passed — verify artifact integrity.
    issues = _verify_artifacts(run_start, head)
    if issues:
        print(
            f"[local-runner] FAIL head-only: "
            f"{len(issues)} artifact integrity issue(s):",
            file=sys.stderr,
        )
        for msg in issues:
            print(f"  {msg}", file=sys.stderr)
        record_executor_mode(
            "local",
            ran=len(HEAD_ONLY_PIPELINE),
            failed=1,
            reason=f"head-only: artifact integrity failed ({len(issues)} issues)",
        )
        return 1

    print(
        f"[local-runner] ok head-only: pipeline passed, "
        f"all artifacts match HEAD={head}"
    )
    record_executor_mode(
        "local",
        ran=len(HEAD_ONLY_PIPELINE),
        failed=0,
        reason=f"head-only: all gates + artifact checks passed for {head}",
    )
    return 0


def main() -> None:
    dry_run = "--dry-run" in sys.argv
    include_pending = "--pending" in sys.argv
    auto = "--auto" in sys.argv
    head_only = "--head-only" in sys.argv
    target_id = ""
    stale_mins = DEFAULT_STALE_MINS

    if "--id" in sys.argv:
        idx = sys.argv.index("--id")
        if idx + 1 < len(sys.argv):
            target_id = sys.argv[idx + 1]

    if "--stale-mins" in sys.argv:
        idx = sys.argv.index("--stale-mins")
        if idx + 1 < len(sys.argv):
            try:
                stale_mins = int(sys.argv[idx + 1])
            except ValueError:
                print(
                    "[local-runner] ERROR: --stale-mins requires an integer",
                    file=sys.stderr,
                )
                sys.exit(2)

    if head_only:
        sys.exit(run_head_only())

    n_failed = run(
        dry_run=dry_run,
        include_pending=include_pending,
        target_id=target_id,
        auto=auto,
        stale_mins=stale_mins,
    )

    if n_failed == 0:
        if not dry_run:
            print("[local-runner] ok: all targeted tasks passed")
        sys.exit(0)
    else:
        print(
            f"[local-runner] {n_failed} task(s) failed",
            file=sys.stderr,
        )
        sys.exit(1)


if __name__ == "__main__":
    main()
