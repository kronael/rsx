#!/usr/bin/env python3
"""Regenerate PROGRESS.md header from acceptance artifacts only.

Single source of truth: artifacts drive PROGRESS, not the other way.

Sources (read-only):
  .ship/tasks.json                       task state snapshot
  rsx-playground/tmp/gate-3-report.json  API gate-3 results
  rsx-playground/tmp/play-artifacts/     Playwright shard results

Writes:
  PROGRESS.md header block (bar + table + proof block)

Divergence detection:
  If PROGRESS.md already contains a header block that disagrees with
  the artifact-derived values, exit 1 and print a diff.  The caller
  must either fix the artifacts or --force to overwrite.

Exit codes:
  0  header written (or already matches)
  1  divergence detected (artifacts vs PROGRESS.md header)
  2  artifact missing / parse error
  3  contradiction in artifacts (done+fail same key)

Usage:
  python3 scripts/publish-progress.py             # check + update
  python3 scripts/publish-progress.py --check     # check only (no write)
  python3 scripts/publish-progress.py --force     # overwrite without check
"""

import json
import re
import subprocess
import sys
import time
from datetime import datetime
from pathlib import Path

ROOT = Path(__file__).parent.parent
PLAYGROUND = ROOT / "rsx-playground"
TMP = PLAYGROUND / "tmp"
PROGRESS = ROOT / "PROGRESS.md"
TASKS_FILE = ROOT / ".ship" / "tasks.json"
GATE3_REPORT = TMP / "gate-3-report.json"
PLAY_ARTIFACT_DIR = TMP / "play-artifacts"
PLAY_SHARDS = ["routing", "htmx-partials", "process-control", "trade-ui"]

# The single canonical total. Denominator must equal this.
CANONICAL_TOTAL = 223


# ── Artifact readers ──────────────────────────────────────────────────

def load_tasks() -> list[dict]:
    if not TASKS_FILE.exists():
        return []
    try:
        data = json.loads(TASKS_FILE.read_text())
        return data if isinstance(data, list) else []
    except Exception as e:
        print(f"[publish-progress] ERROR: tasks.json: {e}", file=sys.stderr)
        sys.exit(2)


def load_gate3() -> dict | None:
    if not GATE3_REPORT.exists():
        return None
    try:
        return json.loads(GATE3_REPORT.read_text())
    except Exception as e:
        print(f"[publish-progress] ERROR: gate-3-report.json: {e}",
              file=sys.stderr)
        sys.exit(2)


def load_playwright() -> dict:
    """Load and aggregate Playwright shard results.

    Returns dict with:
      total_passed, total_failed, shards_present, canonical_ok,
      failing_ids (list of shard::title strings)
    """
    total_passed = 0
    total_failed = 0
    shards_present: list[str] = []
    failing_ids: list[str] = []
    contradictions: list[str] = []

    for shard in PLAY_SHARDS:
        report_file = PLAY_ARTIFACT_DIR / shard / "report.json"
        if not report_file.exists():
            continue
        try:
            data = json.loads(report_file.read_text())
        except Exception as e:
            print(f"[publish-progress] ERROR: play-artifacts/{shard}: {e}",
                  file=sys.stderr)
            sys.exit(2)

        # Contradiction check
        done_set: set[str] = set()
        fail_set: set[str] = set()

        def walk(suites: list) -> None:
            for suite in suites:
                for spec in suite.get("specs", []):
                    title = spec.get("title", "")
                    for test in spec.get("tests", []):
                        if test.get("ok", False):
                            done_set.add(title)
                        if any(
                            r.get("status") == "failed"
                            for r in test.get("results", [])
                        ):
                            fail_set.add(title)
                            failing_ids.append(f"{shard}::{title}")
                walk(suite.get("suites", []))

        walk(data.get("suites", []))

        for title in sorted(done_set & fail_set):
            contradictions.append(
                f"DONE-FAIL [{shard}]: '{title}' ok=true but has failed result"
            )

        stats = data.get("stats", {})
        total_passed += stats.get("expected", 0)
        total_failed += stats.get("unexpected", 0)
        shards_present.append(shard)

    if contradictions:
        print(
            f"[publish-progress] CONTRADICTION: {len(contradictions)} issue(s):",
            file=sys.stderr,
        )
        for c in contradictions:
            print(f"  {c}", file=sys.stderr)
        sys.exit(3)

    canonical_ok = (
        bool(shards_present)
        and total_passed == CANONICAL_TOTAL
        and total_failed == 0
    )
    return {
        "total_passed": total_passed,
        "total_failed": total_failed,
        "shards_present": shards_present,
        "canonical_ok": canonical_ok,
        "failing_ids": failing_ids,
    }


def git_sha() -> str:
    try:
        return subprocess.check_output(
            ["git", "rev-parse", "--short", "HEAD"],
            cwd=ROOT, stderr=subprocess.DEVNULL,
        ).decode().strip()
    except Exception:
        return "unknown"


# ── Section builders ──────────────────────────────────────────────────

def build_bar(passed: int, total: int) -> str:
    """Build progress bar line: [███░░] 45%  100/223"""
    if total == 0:
        return "[░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░]   0%    0/0"
    pct = round(100 * passed / total)
    width = 30
    filled = round(width * passed / total)
    bar = "█" * filled + "░" * (width - filled)
    return f"[{bar}] {pct:3d}%  {passed}/{total}"


def build_header(tasks: list[dict], play: dict, gate3: dict | None) -> str:
    """Build the PROGRESS.md header block from artifacts.

    The header is fully derived from artifacts:
      - completed: playwright passed (or 0 if not yet run)
      - running: tasks with status==running
      - pending: tasks with status==pending (excluding terminal)
      - failed: playwright failed (or tasks failed if no play artifacts)
      - denominator: always CANONICAL_TOTAL (223)

    When playwright shards have been run, the progress bar reflects
    playwright counts (passed/223).  Before shards run, it reflects
    task completion counts.
    """
    # Task counts from snapshot
    task_completed = sum(1 for t in tasks if t.get("status") == "completed")
    task_running = sum(1 for t in tasks if t.get("status") == "running")
    task_pending = sum(1 for t in tasks if t.get("status") == "pending")
    task_failed = sum(1 for t in tasks if t.get("status") == "failed")

    # Playwright counts (authoritative when shards present)
    pw_passed = play["total_passed"]
    pw_failed = play["total_failed"]
    shards_present = play["shards_present"]

    if shards_present:
        # Playwright run: bar = playwright passed/223
        completed = pw_passed
        failed = pw_failed
        # running/pending come from tasks
        running = task_running
        pending = task_pending
    else:
        # No playwright run yet: bar = task completed/223
        completed = task_completed
        running = task_running
        pending = task_pending
        failed = task_failed

    now = datetime.now().strftime("%b %d %H:%M:%S")
    bar = build_bar(completed, CANONICAL_TOTAL)

    # Gate-3 summary
    g3_line = ""
    if gate3 is not None:
        g3_passed = gate3.get("passed", 0)
        g3_failed = gate3.get("failed", 0)
        g3_total = gate3.get("total", 0)
        g3_line = (
            f"\n<!-- gate-3: {g3_passed}/{g3_total} passed"
            f", {g3_failed} failed -->"
        )

    # Proof block
    sha = git_sha()
    if shards_present:
        play_summary = (
            f"playwright: {pw_passed}/{CANONICAL_TOTAL} passed"
            f", {pw_failed} failed"
            f" | shards: {', '.join(shards_present)}"
        )
        canonical_line = (
            f"canonical_ok: {'yes' if play['canonical_ok'] else 'NO'}"
        )
    else:
        play_summary = "playwright: not yet run"
        canonical_line = "canonical_ok: no"

    proof = (
        f"\n<!-- acceptance-proof\n"
        f"commit: {sha}\n"
        f"generated: {now}\n"
        f"{play_summary}\n"
        f"{canonical_line}\n"
        f"-->"
    )

    header = (
        f"# PROGRESS\n\n"
        f"updated: {now}  \n"
        f"phase: {'complete' if play['canonical_ok'] else 'executing'}\n\n"
        f"```\n{bar}\n```\n\n"
        f"| | count |\n"
        f"|---|---|\n"
        f"| completed | {completed} |\n"
        f"| running | {running} |\n"
        f"| pending | {pending} |\n"
        f"| failed | {failed} |"
        f"{g3_line}"
        f"{proof}"
    )
    return header


# ── Divergence check ──────────────────────────────────────────────────

def extract_existing_header(text: str) -> str | None:
    """Extract the header block (everything before ## workers or ## log)."""
    m = re.search(r'^(##\s+workers|##\s+log)', text, re.MULTILINE)
    if m:
        return text[:m.start()].rstrip()
    return text.rstrip()


def headers_diverge(existing: str, generated: str) -> bool:
    """Compare headers ignoring timestamps and commit SHA (volatile fields)."""
    def normalize(s: str) -> str:
        # Remove timestamp lines
        s = re.sub(r'updated:.*', 'updated: TIMESTAMP', s)
        s = re.sub(r'generated:.*', 'generated: TIMESTAMP', s)
        # Remove commit SHA
        s = re.sub(r'commit: [0-9a-f]+', 'commit: SHA', s)
        return s.strip()
    return normalize(existing) != normalize(generated)


# ── Main ──────────────────────────────────────────────────────────────

def main() -> None:
    check_only = "--check" in sys.argv
    force = "--force" in sys.argv

    tasks = load_tasks()
    gate3 = load_gate3()
    play = load_playwright()

    generated_header = build_header(tasks, play, gate3)

    # Read existing PROGRESS.md
    if PROGRESS.exists():
        existing_text = PROGRESS.read_text()
        existing_header = extract_existing_header(existing_text)
        rest = existing_text[len(existing_header):].lstrip("\n")
    else:
        existing_text = ""
        existing_header = ""
        rest = ""

    diverged = headers_diverge(existing_header, generated_header)

    if diverged and not force:
        print(
            "[publish-progress] DIVERGENCE: PROGRESS.md header disagrees "
            "with artifacts",
            file=sys.stderr,
        )
        # Show which numeric fields differ
        def extract_nums(s: str) -> dict[str, str]:
            nums: dict[str, str] = {}
            for label in ("completed", "running", "pending", "failed"):
                m = re.search(rf'\|\s*{label}\s*\|\s*(\d+)', s)
                if m:
                    nums[label] = m.group(1)
            bar = re.search(r'\[[\█░]+\]\s+(\d+)%\s+(\d+)/(\d+)', s)
            if bar:
                nums["bar_pct"] = bar.group(1)
                nums["bar_num"] = bar.group(2)
                nums["bar_den"] = bar.group(3)
            return nums

        existing_nums = extract_nums(existing_header)
        generated_nums = extract_nums(generated_header)
        for k in sorted(set(existing_nums) | set(generated_nums)):
            ev = existing_nums.get(k, "?")
            gv = generated_nums.get(k, "?")
            if ev != gv:
                print(
                    f"  {k}: PROGRESS.md={ev} artifacts={gv}",
                    file=sys.stderr,
                )
        if check_only:
            sys.exit(1)
        print(
            "[publish-progress] use --force to overwrite with artifact values",
            file=sys.stderr,
        )
        sys.exit(1)

    if check_only:
        if not diverged:
            print("[publish-progress] ok: PROGRESS.md header matches artifacts")
        sys.exit(0 if not diverged else 1)

    # Write updated PROGRESS.md
    # Preserve everything after the header (workers, log sections)
    new_text = generated_header + "\n\n" + rest if rest else generated_header + "\n"
    PROGRESS.write_text(new_text)

    pw = play["total_passed"]
    print(
        f"[publish-progress] written: {pw}/{CANONICAL_TOTAL} playwright"
        f", canonical_ok={play['canonical_ok']}"
        f", commit={git_sha()}"
    )
    sys.exit(0)


if __name__ == "__main__":
    main()
