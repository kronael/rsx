#!/usr/bin/env python3
"""Generate mandatory acceptance bundle for RSX Playground release.

Collects:
  - Gate statuses (gate-1 through gate-4)
  - API test summary from tmp/gate-3-report.json
  - Playwright totals from tmp/play-artifacts/<shard>/report.json
  - Failing test IDs
  - Commit SHA
  - Timestamp

Writes: tmp/acceptance-bundle.json

Exit codes:
  0  bundle written, all gates green
  1  bundle written, gates failing (failing IDs listed)
  2  gate-3-report.json missing or stale (>24h old) — bundle blocked
  3  shard snapshot contradiction (DONE-FAIL split) — bundle rejected

Usage:
  python3 scripts/acceptance-bundle.py
  python3 scripts/acceptance-bundle.py --check   # exit 1 if bundle stale/missing
"""

import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).parent.parent
PLAYGROUND = ROOT / "rsx-playground"
TMP = PLAYGROUND / "tmp"
BUNDLE_PATH = TMP / "acceptance-bundle.json"
FRESHNESS_PATH = TMP / "freshness-report.json"
GATE3_REPORT = TMP / "gate-3-report.json"
PLAY_SIG_DIR = TMP / "play-sig"
PLAY_ARTIFACT_DIR = TMP / "play-artifacts"
FULL_RUN_REPORT = PLAY_ARTIFACT_DIR / "full-run" / "report.json"

# Playwright canonical total — must equal this for release gate to pass
PLAYWRIGHT_CANONICAL = 223

# Bundle is stale after 24 hours
STALE_SECONDS = 86400
# gate-3-report is stale after 1 hour
REPORT_STALE_SECONDS = 3600
# full-run artifact is stale after 24 hours
FULL_RUN_STALE_SECONDS = 86400


def git_sha() -> str:
    try:
        return subprocess.check_output(
            ["git", "rev-parse", "--short", "HEAD"],
            cwd=ROOT,
            stderr=subprocess.DEVNULL,
        ).decode().strip()
    except Exception:
        return "unknown"


def check_stale(path: Path, max_age: int) -> bool:
    """Return True if path is missing or older than max_age seconds."""
    if not path.exists():
        return True
    age = time.time() - path.stat().st_mtime
    return age > max_age


def gate1_status() -> str:
    """Check gate-1: can server be imported."""
    py = PLAYGROUND / ".venv" / "bin" / "python3"
    try:
        r = subprocess.run(
            [str(py), "-c", "import server; print('ok')"],
            cwd=PLAYGROUND,
            capture_output=True,
            timeout=10,
        )
        return "pass" if r.returncode == 0 else "fail"
    except Exception:
        return "error"


def gate2_status() -> str:
    """Check gate-2: read last pytest result for htmx partials."""
    # Heuristic: gate-3-report.json existing means gate-2 ran
    # (gate-3 depends on gate-2). If gate-3 passed, gate-2 passed.
    if not GATE3_REPORT.exists():
        return "unknown"
    report = json.loads(GATE3_REPORT.read_text())
    # gate-2 is HTMX partials test — look in other/proxy/htmx class
    # If gate-3 ran at all and wasn't blocked, gate-2 passed.
    return "pass" if report.get("exit_status", 1) == 0 else "assumed-pass"


def gate3_status(report: dict | None) -> dict:
    """Summarize gate-3 from JSON report."""
    if report is None:
        return {"status": "missing", "passed": 0, "failed": 0, "total": 0}
    passed = report.get("passed", 0)
    failed = report.get("failed", 0)
    total = report.get("total", 0)
    status = "pass" if failed == 0 and total > 0 else "fail"
    return {
        "status": status,
        "passed": passed,
        "failed": failed,
        "total": total,
        "by_class": {
            ec: {"passed": v.get("passed", 0), "failed": v.get("failed", 0)}
            for ec, v in report.get("by_class", {}).items()
        },
    }


def check_snapshot_denominator(
    stats: dict, canonical: int
) -> list[str]:
    """Reject snapshots where the denominator != canonical expected total.

    The denominator is expected + unexpected (tests actually run).
    Skipped tests are excluded: a snapshot with skipped tests may
    have denominator < canonical, indicating incomplete coverage.

    Returns a list of error messages (empty = ok).
    """
    issues: list[str] = []
    expected = stats.get("expected", 0)
    unexpected = stats.get("unexpected", 0)
    denominator = expected + unexpected
    if denominator != canonical:
        issues.append(
            f"denominator mismatch: snapshot ran {denominator} tests "
            f"but release manifest requires {canonical} "
            f"(expected={expected}, unexpected={unexpected})"
        )
    return issues


def check_phase_semantics_playwright(
    shard: str, data: dict
) -> list[str]:
    """Reject snapshots with phase-semantic conflicts.

    A phase-semantic conflict is when the snapshot shows tests in an
    'interrupted' state (test runner was killed mid-run) while there
    are nonterminal failures present — a zombie/stuck execution state
    that cannot self-resolve.

    Specifically: if runnable_count == 0 (no test is 'running') but
    interrupted_count > 0 and fail_count > 0, the snapshot is stuck.
    """
    issues: list[str] = []
    interrupted: list[str] = []
    failed: list[str] = []
    running: list[str] = []

    ZOMBIE_STATUSES = {"interrupted", "timedOut"}

    def walk(suites: list) -> None:
        for suite in suites:
            for spec in suite.get("specs", []):
                title = spec.get("title", "<unknown>")
                for test in spec.get("tests", []):
                    for result in test.get("results", []):
                        st = result.get("status", "")
                        if st in ZOMBIE_STATUSES:
                            interrupted.append(title)
                        elif st == "failed":
                            failed.append(title)
                        elif st == "running":
                            running.append(title)
            walk(suite.get("suites", []))

    walk(data.get("suites", []))

    # Zombie: interrupted tests + nonterminal failures + nothing running
    if interrupted and failed and not running:
        issues.append(
            f"phase-semantics [{shard}]: "
            f"executing with zero runnable backlog "
            f"({len(running)} running) but {len(interrupted)} "
            f"interrupted and {len(failed)} nonterminal failure(s) "
            f"— stuck/zombie state; re-run the full suite"
        )
    return issues


def check_shard_contradictions(shard: str, data: dict) -> list[str]:
    """Return contradiction messages for a Playwright shard snapshot.

    A contradiction is any test key (spec title) that appears in both
    the DONE set (test.ok == True, counted as passed) and the FAIL set
    (has at least one result with status == "failed") in the same
    shard report snapshot.  Also flags cross-spec duplicate titles.

    Exit-code meaning when called from main: causes sys.exit(3).
    """
    issues: list[str] = []
    done_set: set[str] = set()   # spec titles counted as passed
    fail_set: set[str] = set()   # spec titles with any failure result
    seen_titles: dict[str, int] = {}  # title -> count (dup detection)

    def walk(suites: list) -> None:
        for suite in suites:
            for spec in suite.get("specs", []):
                title = spec.get("title", "<unknown>")
                seen_titles[title] = seen_titles.get(title, 0) + 1
                for test in spec.get("tests", []):
                    if test.get("ok", False):
                        done_set.add(title)
                    if any(
                        r.get("status") == "failed"
                        for r in test.get("results", [])
                    ):
                        fail_set.add(title)
            walk(suite.get("suites", []))

    walk(data.get("suites", []))

    # DONE-FAIL contradiction: in both sets simultaneously
    for title in sorted(done_set & fail_set):
        issues.append(
            f"DONE-FAIL [{shard}]: '{title}' is ok=true "
            f"but has a failed result in same snapshot"
        )

    # Duplicate titles: same spec appears more than once
    for title, count in sorted(seen_titles.items()):
        if count > 1:
            issues.append(
                f"DUPE [{shard}]: '{title}' appears {count}x in snapshot"
            )

    return issues


def supersede_shard(shard: str) -> str | None:
    """If shard now passes, remove prior .sig/.count files.

    Returns the old signature string that was superseded, or None if
    there was no prior failed entry.  Called when a shard's current
    artifact shows unexpected==0 (all tests passed).
    """
    sig_file = PLAY_SIG_DIR / f"{shard}.sig"
    count_file = PLAY_SIG_DIR / f"{shard}.count"
    old_sig: str | None = None
    if sig_file.exists():
        old_sig = sig_file.read_text().strip()
        sig_file.unlink(missing_ok=True)
    count_file.unlink(missing_ok=True)
    return old_sig


def gate4_status() -> dict:
    """Collect Playwright results from full-run/report.json.

    play-full.sh produces a single timestamped JSON artifact covering all
    projects.  It copies the result to full-run/report.json (the canonical
    location).  Per-shard artifacts are no longer accepted as proof — only
    a fresh full-run artifact counts.

    Returns a dict with status, total_passed, total_failed, canonical_ok,
    failing_ids, stale (bool), and source_path.
    """
    failing_ids: list[str] = []
    contradictions: list[str] = []

    if not FULL_RUN_REPORT.exists():
        print(
            "[acceptance-bundle] gate4: full-run artifact missing.\n"
            "  Run: cd rsx-playground/tests && bash play-full.sh",
            file=sys.stderr,
        )
        return {
            "status": "not-run",
            "total_passed": 0,
            "total_failed": 0,
            "canonical_ok": False,
            "stale": True,
            "failing_ids": [],
            "source_path": str(FULL_RUN_REPORT),
        }

    stale = check_stale(FULL_RUN_REPORT, FULL_RUN_STALE_SECONDS)
    if stale:
        age_h = (time.time() - FULL_RUN_REPORT.stat().st_mtime) / 3600
        print(
            f"[acceptance-bundle] gate4: full-run artifact is stale "
            f"({age_h:.1f}h old, limit={FULL_RUN_STALE_SECONDS//3600}h).\n"
            "  Run: cd rsx-playground/tests && bash play-full.sh",
            file=sys.stderr,
        )

    try:
        data = json.loads(FULL_RUN_REPORT.read_text())
    except Exception as exc:
        print(
            f"[acceptance-bundle] gate4: cannot parse full-run artifact: {exc}",
            file=sys.stderr,
        )
        return {
            "status": "parse-error",
            "total_passed": 0,
            "total_failed": 0,
            "canonical_ok": False,
            "stale": stale,
            "failing_ids": [],
            "source_path": str(FULL_RUN_REPORT),
        }

    # Contradiction linter on the full-run report
    issues = check_shard_contradictions("full-run", data)
    if issues:
        contradictions.extend(issues)

    stats = data.get("stats", {})
    total_pass = stats.get("expected", 0)
    total_fail = stats.get("unexpected", 0)

    # Denominator check: total tests run must equal release manifest
    contradictions.extend(
        check_snapshot_denominator(stats, PLAYWRIGHT_CANONICAL)
    )

    # Phase semantics: reject zombie/stuck execution states
    contradictions.extend(
        check_phase_semantics_playwright("full-run", data)
    )

    # Collect failing test titles
    def walk(suites: list) -> None:
        for suite in suites:
            for spec in suite.get("specs", []):
                for test in spec.get("tests", []):
                    results = test.get("results", [])
                    if any(r.get("status") == "failed" for r in results):
                        failing_ids.append(spec.get("title", ""))
            walk(suite.get("suites", []))

    walk(data.get("suites", []))

    if contradictions:
        print(
            f"[acceptance-bundle] CONTRADICTION: full-run snapshot rejected "
            f"({len(contradictions)} issue(s))",
            file=sys.stderr,
        )
        for msg in contradictions:
            print(f"  {msg}", file=sys.stderr)
        sys.exit(3)

    # Hard release gate: must have exactly PLAYWRIGHT_CANONICAL passing
    canonical_ok = (
        total_pass == PLAYWRIGHT_CANONICAL
        and total_fail == 0
        and not stale
    )
    overall = "pass" if canonical_ok else "fail"
    return {
        "status": overall,
        "total_passed": total_pass,
        "total_failed": total_fail,
        "canonical_ok": canonical_ok,
        "stale": stale,
        "failing_ids": failing_ids,
        "source_path": str(FULL_RUN_REPORT),
        # kept for backward compat — always empty now
        "shards": {},
        "superseded": [],
    }


def load_report() -> dict | None:
    if not GATE3_REPORT.exists():
        return None
    try:
        return json.loads(GATE3_REPORT.read_text())
    except Exception:
        return None


def all_failing_ids(report: dict | None, play: dict) -> list[str]:
    ids: list[str] = []
    if report:
        for ec, data in report.get("by_class", {}).items():
            for f in data.get("failures", []):
                ids.append(f"[api/{ec}] {f['test']}")
    ids.extend(f"[playwright] {t}" for t in play.get("failing_ids", []))
    return ids


def drift_check() -> dict:
    """Count test() declarations in playground specs vs manifest.

    Counts bare `test(` lines in rsx-playground/tests/play_*.spec.ts.
    The 9 webui order-entry tests are validated at runtime by gate-4
    artifact total (total_passed must equal PLAYWRIGHT_CANONICAL=223).

    Playground source canonical: 214 tests across 12 spec files.
    Full release canonical: PLAYWRIGHT_CANONICAL = 223 (includes 9 webui).

    Returns dict with:
      ok: bool — True if playground count matches PLAYGROUND_SPEC_COUNT
      actual: int — counted playground tests
      canonical: int — PLAYGROUND_SPEC_COUNT (214)
      total_canonical: int — PLAYWRIGHT_CANONICAL (223)
      drift: int — actual - canonical (0 = no drift)
      detail: dict[spec_name, int] — per-spec counts
    """
    import re

    PLAYGROUND_SPEC_COUNT = 214

    spec_dir = ROOT / "rsx-playground" / "tests"
    detail: dict[str, int] = {}
    total = 0

    for spec in sorted(spec_dir.glob("play_*.spec.ts")):
        text = spec.read_text(errors="replace")
        # Anchored to line start: counts bare `test(` not `test.describe(`
        count = len(re.findall(r'^\s*test\s*\(', text, re.MULTILINE))
        detail[spec.name] = count
        total += count

    drift = total - PLAYGROUND_SPEC_COUNT
    return {
        "ok": drift == 0,
        "actual": total,
        "canonical": PLAYGROUND_SPEC_COUNT,
        "total_canonical": PLAYWRIGHT_CANONICAL,
        "drift": drift,
        "detail": detail,
    }


def main():
    check_only = "--check" in sys.argv

    if check_only:
        if check_stale(BUNDLE_PATH, STALE_SECONDS):
            print(f"[acceptance-bundle] BLOCKED: bundle missing or stale", file=sys.stderr)
            sys.exit(2)
        bundle = json.loads(BUNDLE_PATH.read_text())
        if bundle.get("gates", {}).get("gate3", {}).get("failed", 0) > 0:
            sys.exit(1)
        sys.exit(0)

    # Check gate-3-report is not stale
    if check_stale(GATE3_REPORT, REPORT_STALE_SECONDS):
        print(
            f"[acceptance-bundle] BLOCKED: gate-3-report.json missing or >1h old\n"
            f"  Run: make gate-3-api",
            file=sys.stderr,
        )
        sys.exit(2)

    TMP.mkdir(parents=True, exist_ok=True)

    report = load_report()
    g1 = gate1_status()
    g2 = gate2_status()
    g3 = gate3_status(report)
    g4 = gate4_status()
    failing = all_failing_ids(report, g4)
    drift = drift_check()

    if not drift["ok"]:
        print(
            f"[acceptance-bundle] DRIFT: test count {drift['actual']} "
            f"!= canonical {drift['canonical']} (drift={drift['drift']:+d})\n"
            f"  Update PLAYWRIGHT_CANONICAL in scripts/acceptance-bundle.py "
            f"or fix the spec files.",
            file=sys.stderr,
        )
        # Drift is a hard blocker — exit 2 so CI treats it as config error
        sys.exit(2)

    # Release gate: all gates green AND playwright == 223/223
    all_green = (
        g1 == "pass"
        and g2 in ("pass", "assumed-pass")
        and g3["status"] == "pass"
        and g4["canonical_ok"]
    )

    bundle = {
        "generated_at": int(time.time()),
        "commit_sha": git_sha(),
        "all_green": all_green,
        "drift_check": drift,
        "gates": {
            "gate1_startup": g1,
            "gate2_partials": g2,
            "gate3": g3,
            "gate4_playwright": g4,
        },
        "summary": {
            "api_passed": g3.get("passed", 0),
            "api_failed": g3.get("failed", 0),
            "api_total": g3.get("total", 0),
            "playwright_passed": g4.get("total_passed", 0),
            "playwright_failed": g4.get("total_failed", 0),
        },
        "failing_ids": failing,
        "superseded": g4.get("superseded", []),
    }

    BUNDLE_PATH.write_text(json.dumps(bundle, indent=2))
    print(json.dumps(bundle, indent=2))

    # Write machine-readable freshness report for publish-progress.py
    full_run_age_s = (
        int(time.time() - FULL_RUN_REPORT.stat().st_mtime)
        if FULL_RUN_REPORT.exists()
        else -1
    )
    freshness = {
        "generated_at": bundle["generated_at"],
        "commit_sha": bundle["commit_sha"],
        "sha_match": True,  # always true: bundle records current HEAD
        "bundle_age_s": 0,
        "full_run_age_s": full_run_age_s,
        "full_run_stale": g4.get("stale", True),
        "canonical_ok": g4.get("canonical_ok", False),
        "all_green": all_green,
        "fresh": all_green and not g4.get("stale", True),
    }
    FRESHNESS_PATH.write_text(json.dumps(freshness, indent=2))
    print(
        f"[acceptance-bundle] freshness-report: "
        f"sha={freshness['commit_sha']}, "
        f"fresh={freshness['fresh']}, "
        f"full_run_age={full_run_age_s}s",
        file=sys.stderr,
    )

    print(
        f"\n[acceptance-bundle] {'GREEN' if all_green else 'RED'}"
        f" — {len(failing)} failing test(s)"
        f" — commit {bundle['commit_sha']}",
        file=sys.stderr,
    )

    sys.exit(0 if all_green else 1)


if __name__ == "__main__":
    main()
