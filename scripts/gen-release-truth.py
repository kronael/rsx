#!/usr/bin/env python3
"""Generate release_truth.json for current HEAD from acceptance-bundle.json.

Reads git SHA directly from .git directory (no external CLI dependency).
Reads gate states, API summary, Playwright totals, and failing IDs from
rsx-playground/tmp/acceptance-bundle.json.

Writes: rsx-playground/tmp/release_truth.json

Exit codes:
  0  written successfully
  2  acceptance-bundle.json missing, stale (>24h), or SHA mismatch
  3  bundle structurally invalid
"""

import json
import sys
import time
from pathlib import Path

ROOT = Path(__file__).parent.parent
PLAYGROUND = ROOT / "rsx-playground"
TMP = PLAYGROUND / "tmp"
BUNDLE_PATH = TMP / "acceptance-bundle.json"
OUT_PATH = TMP / "release_truth.json"

CANONICAL_TOTAL = 421
BUNDLE_STALE_SECONDS = 86400


def git_sha_from_fs(root: Path) -> str:
    """Read current HEAD SHA from .git dir — no subprocess."""
    git_dir = root / ".git"
    try:
        head = (git_dir / "HEAD").read_text().strip()
        if head.startswith("ref: "):
            ref = head[5:]  # e.g. refs/heads/master
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


def load_bundle() -> dict:
    if not BUNDLE_PATH.exists():
        print(
            "[gen-release-truth] BLOCKED: acceptance-bundle.json missing.\n"
            "  Run: make acceptance-bundle",
            file=sys.stderr,
        )
        sys.exit(2)

    age = time.time() - BUNDLE_PATH.stat().st_mtime
    if age > BUNDLE_STALE_SECONDS:
        print(
            f"[gen-release-truth] BLOCKED: acceptance-bundle.json stale"
            f" ({age / 3600:.1f}h old).\n"
            "  Run: make acceptance-bundle",
            file=sys.stderr,
        )
        sys.exit(2)

    try:
        return json.loads(BUNDLE_PATH.read_text())
    except Exception as exc:
        print(
            f"[gen-release-truth] BLOCKED: cannot parse bundle: {exc}",
            file=sys.stderr,
        )
        sys.exit(2)


def main() -> None:
    sha = git_sha_from_fs(ROOT)
    bundle = load_bundle()

    bundle_sha = bundle.get("commit_sha", "unknown")
    if bundle_sha not in ("unknown",) and bundle_sha != sha:
        print(
            f"[gen-release-truth] BLOCKED: SHA mismatch.\n"
            f"  bundle commit_sha : {bundle_sha}\n"
            f"  current HEAD      : {sha}\n"
            "  Run: make acceptance-bundle",
            file=sys.stderr,
        )
        sys.exit(2)

    gates = bundle.get("gates", {})
    g3 = gates.get("gate3", {})
    g4 = gates.get("gate4_playwright", {})

    truth = {
        "generated_at": int(time.time()),
        "commit_sha": sha,
        "gate1_startup": gates.get("gate1_startup", "unknown"),
        "gate2_partials": gates.get("gate2_partials", "unknown"),
        "gate3": {
            "status": g3.get("status", "unknown"),
            "passed": g3.get("passed", 0),
            "failed": g3.get("failed", 0),
            "total": g3.get("total", 0),
        },
        "playwright_passed": g4.get("total_passed", 0),
        "playwright_failed": g4.get("total_failed", 0),
        "playwright_canonical": CANONICAL_TOTAL,
        "canonical_ok": g4.get("canonical_ok", False),
        "all_green": bundle.get("all_green", False),
        "failing_ids": bundle.get("failing_ids", []),
    }

    TMP.mkdir(parents=True, exist_ok=True)
    OUT_PATH.write_text(json.dumps(truth, indent=2))

    pw = truth["playwright_passed"]
    print(
        f"[gen-release-truth] written: sha={sha}"
        f" playwright={pw}/{CANONICAL_TOTAL}"
        f" all_green={truth['all_green']}"
    )


if __name__ == "__main__":
    main()
