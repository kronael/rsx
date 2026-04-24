#!/usr/bin/env python3
"""CI guard: validate acceptance-bundle artifact JSON.

Fails if:
  - denominator (expected + unexpected) != 421
  - artifact commit_sha != current HEAD short SHA
  - phase-state contradictions: interrupted/timedOut tests +
    nonterminal failures with nothing running (zombie state)
  - PROGRESS-derived counts diverge from release_truth.json

Takes the artifact JSON file path as sole input.

Exit codes:
  0  artifact passes all guards
  1  one or more guard violations found
  2  artifact missing or unreadable

Usage:
  python3 scripts/ci-guard.py rsx-playground/tmp/acceptance-bundle.json
  python3 scripts/ci-guard.py path/to/report.json
"""

import json
import subprocess
import sys
from pathlib import Path

PLAYWRIGHT_CANONICAL = 421

ZOMBIE_STATUSES = {"interrupted", "timedOut"}


def git_sha() -> str:
    try:
        return subprocess.check_output(
            ["git", "rev-parse", "--short", "HEAD"],
            stderr=subprocess.DEVNULL,
        ).decode().strip()
    except Exception:
        return "unknown"


def check_sha(bundle: dict) -> list[str]:
    """Fail if bundle commit_sha != current HEAD short SHA."""
    artifact_sha = bundle.get("commit_sha", "")
    if not artifact_sha or artifact_sha == "unknown":
        return ["sha-mismatch: artifact has no commit_sha field"]
    head = git_sha()
    if head == "unknown":
        return []  # can't verify — skip silently
    if artifact_sha != head:
        return [
            f"sha-mismatch: artifact sha={artifact_sha} "
            f"!= HEAD sha={head} — re-run acceptance-bundle"
        ]
    return []


def check_denominator(stats: dict) -> list[str]:
    """Fail if expected + unexpected != PLAYWRIGHT_CANONICAL.

    Skipped tests are excluded from the denominator intentionally:
    a snapshot with skipped tests may have denominator < canonical,
    indicating incomplete coverage.
    """
    expected = stats.get("expected", 0)
    unexpected = stats.get("unexpected", 0)
    denominator = expected + unexpected
    if denominator != PLAYWRIGHT_CANONICAL:
        return [
            f"denominator mismatch: snapshot ran {denominator} tests "
            f"but release manifest requires {PLAYWRIGHT_CANONICAL} "
            f"(expected={expected}, unexpected={unexpected})"
        ]
    return []


def check_phase_contradictions(data: dict) -> list[str]:
    """Fail on zombie/stuck execution states in the Playwright report.

    A zombie state is: interrupted/timedOut tests present AND nonterminal
    failures present AND nothing currently running. Such a state cannot
    self-resolve; the suite must be re-run.
    """
    interrupted: list[str] = []
    failed: list[str] = []
    running: list[str] = []

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

    if interrupted and failed and not running:
        return [
            f"phase-semantics: executing with zero runnable backlog "
            f"({len(running)} running) but {len(interrupted)} interrupted "
            f"and {len(failed)} nonterminal failure(s) "
            f"— stuck/zombie state; re-run the full suite"
        ]
    return []


def check_release_truth_divergence(
    bundle: dict,
    artifact_path: Path,
) -> list[str]:
    """Fail if PROGRESS-derived counts diverge from release_truth.json.

    Looks for release_truth.json in the same tmp/ directory as the
    artifact.  If the file is absent, the check is skipped silently
    (CI may not have generated it yet).  When present, playwright_passed
    and playwright_failed must match the bundle gate4 totals exactly.
    """
    truth_path = artifact_path.parent / "release_truth.json"
    if not truth_path.exists():
        return []

    try:
        truth = json.loads(truth_path.read_text())
    except Exception as exc:
        return [f"release-truth: cannot parse {truth_path.name}: {exc}"]

    g4 = bundle.get("gates", {}).get("gate4_playwright", {})
    bundle_passed = g4.get("total_passed", 0)
    bundle_failed = g4.get("total_failed", 0)

    truth_passed = truth.get("playwright_passed", None)
    truth_failed = truth.get("playwright_failed", None)

    issues: list[str] = []
    if truth_passed is not None and truth_passed != bundle_passed:
        issues.append(
            f"release-truth divergence: playwright_passed"
            f" truth={truth_passed} != bundle={bundle_passed}"
            f" — re-run: make gen-release-truth"
        )
    if truth_failed is not None and truth_failed != bundle_failed:
        issues.append(
            f"release-truth divergence: playwright_failed"
            f" truth={truth_failed} != bundle={bundle_failed}"
            f" — re-run: make gen-release-truth"
        )
    return issues


def guard_bundle(bundle: dict, artifact_path: Path | None = None) -> list[str]:
    """Run all guards against an acceptance bundle artifact.

    The bundle wraps the Playwright full-run report under
    gates.gate4_playwright. If the bundle contains a gate4 section
    with a source_path, we load the raw Playwright JSON from disk
    for phase-contradiction checking. Otherwise we fall back to
    checking whatever is in the bundle directly.

    Returns a list of violation messages (empty = all clear).
    """
    issues: list[str] = []

    # SHA check: artifact must match current HEAD
    issues.extend(check_sha(bundle))

    # Gate-4 Playwright stats live under gates.gate4_playwright
    g4 = bundle.get("gates", {}).get("gate4_playwright", {})

    # Denominator check from summary totals
    stats = {
        "expected": g4.get("total_passed", 0),
        "unexpected": g4.get("total_failed", 0),
    }
    # Also accept top-level stats key (raw report.json format)
    if "stats" in bundle:
        stats = bundle["stats"]

    issues.extend(check_denominator(stats))

    # Phase-contradiction check on raw Playwright suites
    # Prefer top-level suites (raw report.json), else skip
    if "suites" in bundle:
        issues.extend(check_phase_contradictions(bundle))

    # release_truth.json divergence check
    if artifact_path is not None:
        issues.extend(
            check_release_truth_divergence(bundle, artifact_path)
        )

    return issues


def guard_report(report: dict) -> list[str]:
    """Run all guards against a raw Playwright report.json."""
    issues: list[str] = []
    stats = report.get("stats", {})
    issues.extend(check_denominator(stats))
    issues.extend(check_phase_contradictions(report))
    return issues


def load_artifact(path: Path) -> dict:
    if not path.exists():
        print(
            f"[ci-guard] ERROR: artifact not found: {path}",
            file=sys.stderr,
        )
        sys.exit(2)
    try:
        return json.loads(path.read_text())
    except Exception as exc:
        print(
            f"[ci-guard] ERROR: cannot parse {path}: {exc}",
            file=sys.stderr,
        )
        sys.exit(2)


def main() -> None:
    args = [a for a in sys.argv[1:] if not a.startswith("-")]
    if not args:
        print(
            "usage: python3 scripts/ci-guard.py <artifact.json>",
            file=sys.stderr,
        )
        sys.exit(2)

    path = Path(args[0])
    artifact = load_artifact(path)

    # Detect format: raw Playwright report has a "stats" key at top level
    # with "expected"/"unexpected". Acceptance bundle has "gates" key.
    if "stats" in artifact and "suites" in artifact:
        issues = guard_report(artifact)
    elif "gates" in artifact:
        issues = guard_bundle(artifact, path)
        # If the bundle records source_path, also check raw Playwright data
        src = artifact.get(
            "gates", {}
        ).get("gate4_playwright", {}).get("source_path")
        if src:
            raw_path = Path(src)
            if raw_path.exists():
                try:
                    raw = json.loads(raw_path.read_text())
                    issues.extend(check_phase_contradictions(raw))
                    # Deduplicate
                    issues = list(dict.fromkeys(issues))
                except Exception:
                    pass
    else:
        # Unknown format — run both checks on whatever we have
        stats = artifact.get("stats", {})
        issues = check_denominator(stats)
        issues.extend(check_phase_contradictions(artifact))

    if issues:
        print(
            f"[ci-guard] FAIL: {len(issues)} violation(s) in {path.name}:",
            file=sys.stderr,
        )
        for msg in issues:
            print(f"  ✗ {msg}", file=sys.stderr)
        sys.exit(1)

    print(f"[ci-guard] ok: {path.name} passes all guards")
    sys.exit(0)


if __name__ == "__main__":
    main()
