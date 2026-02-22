#!/usr/bin/env python3
"""Regenerate PROGRESS.md header from acceptance-bundle.json only.

Single source of truth: acceptance-bundle.json drives PROGRESS.
Never reads shard reports, gate-3-report, or tasks.json directly.

Requires:
  rsx-playground/tmp/acceptance-bundle.json (fresh, <24h, SHA matches HEAD)
  Run `make acceptance-bundle` first.

Writes:
  PROGRESS.md header block (bar + table + proof block)

Divergence detection:
  If PROGRESS.md already contains a header block that disagrees with
  the artifact-derived values, exit 1 and print a diff.  The caller
  must either fix the artifacts or --force to overwrite.

Exit codes:
  0  header written (or already matches)
  1  divergence detected (artifacts vs PROGRESS.md header)
  2  artifact missing / stale / parse error / SHA mismatch
  3  bundle structurally invalid (missing required fields)

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
BUNDLE_PATH = TMP / "acceptance-bundle.json"
TRUTH_PATH = TMP / "release_truth.json"

# The single canonical total.
CANONICAL_TOTAL = 223

# Bundle is stale after 24 hours.
BUNDLE_STALE_SECONDS = 86400
# release_truth.json is stale after 24 hours.
TRUTH_STALE_SECONDS = 86400

REQUIRED_BUNDLE_FIELDS = [
    "generated_at",
    "commit_sha",
    "all_green",
    "gates",
    "summary",
    "failing_ids",
    "drift_check",
]

REQUIRED_GATE_KEYS = [
    "gate1_startup",
    "gate2_partials",
    "gate3",
    "gate4_playwright",
]


# ── Helpers ───────────────────────────────────────────────────────────

def git_sha_from_fs(root: Path) -> str:
    """Read current HEAD SHA from .git dir — no subprocess."""
    git_dir = root / ".git"
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


def git_sha() -> str:
    try:
        return subprocess.check_output(
            ["git", "rev-parse", "--short", "HEAD"],
            cwd=ROOT, stderr=subprocess.DEVNULL,
        ).decode().strip()
    except Exception:
        return git_sha_from_fs(ROOT)


def check_release_truth() -> None:
    """Fail (exit 2) if release_truth.json is missing, stale, or SHA stale.

    Called before bundle load so that publication is blocked whenever
    gen-release-truth has not been run against current HEAD.
    """
    if not TRUTH_PATH.exists():
        print(
            "[publish-progress] BLOCKED: release_truth.json missing.\n"
            "  Run: make gen-release-truth",
            file=sys.stderr,
        )
        sys.exit(2)

    age = time.time() - TRUTH_PATH.stat().st_mtime
    if age > TRUTH_STALE_SECONDS:
        print(
            f"[publish-progress] BLOCKED: release_truth.json stale"
            f" ({age / 3600:.1f}h old).\n"
            "  Run: make gen-release-truth",
            file=sys.stderr,
        )
        sys.exit(2)

    try:
        truth = json.loads(TRUTH_PATH.read_text())
    except Exception as exc:
        print(
            f"[publish-progress] BLOCKED: cannot parse"
            f" release_truth.json: {exc}",
            file=sys.stderr,
        )
        sys.exit(2)

    current = git_sha_from_fs(ROOT)
    truth_sha = truth.get("commit_sha", "unknown")
    if truth_sha not in ("unknown",) and truth_sha != current:
        print(
            f"[publish-progress] BLOCKED: release_truth.json SHA mismatch.\n"
            f"  release_truth commit_sha : {truth_sha}\n"
            f"  current HEAD             : {current}\n"
            "  Run: make gen-release-truth",
            file=sys.stderr,
        )
        sys.exit(2)


def build_bar(passed: int, total: int) -> str:
    """Build progress bar: [███░░] 45%  100/223"""
    if total == 0:
        return "[░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░]   0%    0/0"
    pct = round(100 * passed / total)
    width = 30
    filled = round(width * passed / total)
    bar = "█" * filled + "░" * (width - filled)
    return f"[{bar}] {pct:3d}%  {passed}/{total}"


# ── Bundle loading and validation ─────────────────────────────────────

def validate_bundle_fields(bundle: dict) -> list[str]:
    """Return list of error messages for missing required fields.

    An empty list means the bundle is structurally valid.
    """
    errors: list[str] = []
    for field in REQUIRED_BUNDLE_FIELDS:
        if field not in bundle:
            errors.append(f"missing required field: '{field}'")
    gates = bundle.get("gates")
    if isinstance(gates, dict):
        for key in REQUIRED_GATE_KEYS:
            if key not in gates:
                errors.append(f"missing gates field: '{key}'")
    elif "gates" in bundle:
        errors.append("'gates' must be a dict")
    return errors


def load_bundle() -> dict:
    """Load, validate, and return acceptance-bundle.json.

    Exits with code 2 on any IO/parse/staleness/SHA error.
    Exits with code 3 on structural validation failure.
    """
    if not BUNDLE_PATH.exists():
        print(
            "[publish-progress] BLOCKED: acceptance-bundle.json missing.\n"
            "  Run: make acceptance-bundle",
            file=sys.stderr,
        )
        sys.exit(2)

    age = time.time() - BUNDLE_PATH.stat().st_mtime
    if age > BUNDLE_STALE_SECONDS:
        age_h = age / 3600
        print(
            f"[publish-progress] BLOCKED: acceptance-bundle.json is stale"
            f" ({age_h:.1f}h old, limit="
            f"{BUNDLE_STALE_SECONDS // 3600}h).\n"
            "  Run: make acceptance-bundle",
            file=sys.stderr,
        )
        sys.exit(2)

    try:
        bundle = json.loads(BUNDLE_PATH.read_text())
    except Exception as exc:
        print(
            f"[publish-progress] BLOCKED: cannot parse bundle: {exc}",
            file=sys.stderr,
        )
        sys.exit(2)

    # Structural validation
    errors = validate_bundle_fields(bundle)
    if errors:
        for err in errors:
            print(f"[publish-progress] INVALID: {err}", file=sys.stderr)
        sys.exit(3)

    # SHA freshness: bundle must match current HEAD
    current = git_sha()
    bundle_sha = bundle.get("commit_sha", "unknown")
    if bundle_sha not in ("unknown",) and bundle_sha != current:
        print(
            f"[publish-progress] BLOCKED: SHA mismatch.\n"
            f"  bundle commit_sha : {bundle_sha}\n"
            f"  current HEAD      : {current}\n"
            "  Run: make acceptance-bundle",
            file=sys.stderr,
        )
        sys.exit(2)

    return bundle


# ── Header builder ────────────────────────────────────────────────────

def build_header(bundle: dict) -> str:
    """Build PROGRESS.md header block from acceptance-bundle only."""
    now = datetime.now().strftime("%b %d %H:%M:%S")
    sha = bundle.get("commit_sha", git_sha())

    g4 = bundle.get("gates", {}).get("gate4_playwright", {})
    pw_passed = g4.get("total_passed", 0)
    pw_failed = g4.get("total_failed", 0)
    canonical_ok = g4.get("canonical_ok", False)

    g3 = bundle.get("gates", {}).get("gate3", {})
    g3_passed = g3.get("passed", 0)
    g3_failed = g3.get("failed", 0)
    g3_total = g3.get("total", 0)

    g1 = bundle.get("gates", {}).get("gate1_startup", "?")
    g2 = bundle.get("gates", {}).get("gate2_partials", "?")

    failing_ids: list[str] = bundle.get("failing_ids", [])
    all_green = bundle.get("all_green", False)
    bundle_ts = bundle.get("generated_at", 0)
    bundle_age_s = int(time.time()) - bundle_ts if bundle_ts else -1

    bar = build_bar(pw_passed, CANONICAL_TOTAL)

    # Gate status line
    gate_line = (
        f"\n<!-- gates:"
        f" g1={g1}"
        f" g2={g2}"
        f" g3={g3_passed}/{g3_total}"
        f" g4={pw_passed}/{CANONICAL_TOTAL}"
        f" -->"
    )

    # API summary comment
    api_line = (
        f"\n<!-- api: passed={g3_passed}"
        f", failed={g3_failed}"
        f", total={g3_total} -->"
    )

    # Failing test IDs (truncated to first 20 for readability)
    if failing_ids:
        shown = failing_ids[:20]
        tail = (
            f"\n  ... +{len(failing_ids) - 20} more"
            if len(failing_ids) > 20 else ""
        )
        fail_lines = "\n".join(f"  {t}" for t in shown) + tail
        fail_block = f"\n<!-- failing_ids:\n{fail_lines}\n-->"
    else:
        fail_block = "\n<!-- failing_ids: none -->"

    # Proof block (human-readable summary for release audit)
    proof = (
        f"\n<!-- release-truth\n"
        f"commit: {sha}\n"
        f"generated: {now}\n"
        f"bundle_age: {bundle_age_s}s\n"
        f"playwright: {pw_passed}/{CANONICAL_TOTAL} passed"
        f", {pw_failed} failed\n"
        f"api: {g3_passed}/{g3_total} passed"
        f", {g3_failed} failed\n"
        f"gate1_startup: {g1}\n"
        f"gate2_partials: {g2}\n"
        f"canonical_ok: {'yes' if canonical_ok else 'NO'}\n"
        f"all_green: {'yes' if all_green else 'NO'}\n"
        f"-->"
    )

    header = (
        f"# PROGRESS\n\n"
        f"updated: {now}  \n"
        f"phase: {'complete' if canonical_ok else 'executing'}\n\n"
        f"```\n{bar}\n```\n\n"
        f"| | count |\n"
        f"|---|---|\n"
        f"| completed | {pw_passed} |\n"
        f"| running | 0 |\n"
        f"| pending | 0 |\n"
        f"| failed | {pw_failed} |"
        f"{gate_line}"
        f"{api_line}"
        f"{fail_block}"
        f"{proof}"
    )
    return header


# ── Divergence check ──────────────────────────────────────────────────

def extract_existing_header(text: str) -> str:
    """Extract the header block (everything before ## workers or ## log)."""
    m = re.search(r'^(##\s+workers|##\s+log)', text, re.MULTILINE)
    if m:
        return text[:m.start()].rstrip()
    return text.rstrip()


def headers_diverge(existing: str, generated: str) -> bool:
    """Compare headers ignoring volatile fields (timestamps, SHA)."""
    def normalize(s: str) -> str:
        s = re.sub(r'updated:.*', 'updated: TIMESTAMP', s)
        s = re.sub(r'generated:.*', 'generated: TIMESTAMP', s)
        s = re.sub(r'bundle_age:.*', 'bundle_age: AGE', s)
        s = re.sub(r'commit: [0-9a-f]+', 'commit: SHA', s)
        return s.strip()
    return normalize(existing) != normalize(generated)


# ── Main ──────────────────────────────────────────────────────────────

def main() -> None:
    check_only = "--check" in sys.argv
    force = "--force" in sys.argv

    check_release_truth()
    bundle = load_bundle()
    generated_header = build_header(bundle)

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
            "with acceptance-bundle.json",
            file=sys.stderr,
        )

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
                    f"  {k}: PROGRESS.md={ev}"
                    f" acceptance-bundle={gv}",
                    file=sys.stderr,
                )
        if check_only:
            sys.exit(1)
        print(
            "[publish-progress] use --force to overwrite with bundle values",
            file=sys.stderr,
        )
        sys.exit(1)

    if check_only:
        if not diverged:
            print(
                "[publish-progress] ok: PROGRESS.md header matches "
                "acceptance-bundle.json"
            )
        sys.exit(0 if not diverged else 1)

    # Write updated PROGRESS.md
    new_text = (
        generated_header + "\n\n" + rest if rest
        else generated_header + "\n"
    )
    PROGRESS.write_text(new_text)

    g4 = bundle.get("gates", {}).get("gate4_playwright", {})
    pw = g4.get("total_passed", 0)
    canonical_ok = g4.get("canonical_ok", False)
    print(
        f"[publish-progress] written: {pw}/{CANONICAL_TOTAL} playwright"
        f", canonical_ok={canonical_ok}"
        f", commit={bundle.get('commit_sha', '?')}"
    )
    sys.exit(0)


if __name__ == "__main__":
    main()
