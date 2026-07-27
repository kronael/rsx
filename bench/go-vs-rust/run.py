#!/usr/bin/env python3
"""Build both sides, run each case with warm-up, report medians.

Usage: ./run.py [--reps N] [--cases A,B] [--quick]
Every binary prints one RESULT line (key=value pairs); this script discards
one warm-up run, keeps `reps` measured runs, verifies the Go and Rust
checksums match per workload across ALL runs (proof both sides did identical
work), and prints a markdown table where each cell is the per-metric median
across the measured runs (runs are deterministic in work, so metrics are
independently comparable; tails especially need per-metric medians).
"""

import argparse
import os
import statistics
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.join(HERE, "bin")

# (case, label, binary, extra env)
DEFAULT_MATRIX = [
    ("A", "go @3 bursts", "go-casea", {}),
    ("A", "rust(tokio) @3 bursts", "rust-casea", {}),
    ("A", "rust(tokio+block_in_place) @3", "rust-casea", {"BENCH_MITIGATE": "1"}),
    ("A", "go @6 bursts", "go-casea", {"BENCH_HEAVY_EVERY": "833"}),
    ("A", "rust(tokio) @6 bursts", "rust-casea", {"BENCH_HEAVY_EVERY": "833"}),
    ("A", "rust(tokio+block_in_place) @6", "rust-casea", {"BENCH_HEAVY_EVERY": "833", "BENCH_MITIGATE": "1"}),
    ("B", "go", "go-caseb", {}),
    ("B", "go GOGC=1000", "go-caseb", {"GOGC": "1000"}),
    ("B", "rust(threads)", "rust-caseb", {}),
    ("P1", "go", "go-pingpong", {}),
    ("P1", "rust(tokio)", "rust-pingpong", {}),
    ("P2", "go", "go-sleepers", {}),
    ("P2", "rust(tokio)", "rust-sleepers", {}),
]

COLUMNS = [
    ("thr_ops_s", "thr ops/s"),
    ("p50_ns", "p50"),
    ("p99_ns", "p99"),
    ("p999_ns", "p99.9"),
    ("max_ns", "max"),
    ("rss_peak_mb", "peak RSS"),
    ("cpu_cores", "cores used"),
]


def build():
    subprocess.run(
        ["go", "build", "-o", BIN + "/", "./..."],
        cwd=os.path.join(HERE, "go"), check=True,
    )
    subprocess.run(
        ["cargo", "build", "--release", "--quiet"],
        cwd=os.path.join(HERE, "rust"), check=True,
    )
    for name in ("casea", "caseb", "pingpong", "sleepers"):
        src = os.path.join(HERE, "rust", "target", "release", name)
        dst = os.path.join(BIN, "rust-" + name)
        if os.path.exists(src):
            subprocess.run(["cp", src, dst], check=True)
    for name in ("casea", "caseb", "pingpong", "sleepers"):
        src = os.path.join(BIN, name)
        dst = os.path.join(BIN, "go-" + name)
        if os.path.exists(src):
            os.replace(src, dst)


def run_once(binary, extra_env):
    env = dict(os.environ, **extra_env)
    out = subprocess.run(
        [os.path.join(BIN, binary)], env=env, check=True,
        capture_output=True, text=True,
    ).stdout
    for line in out.splitlines():
        if line.startswith("RESULT "):
            return dict(kv.split("=", 1) for kv in line.split()[1:])
    sys.exit(f"no RESULT line from {binary}")


def fmt_ns(ns):
    ns = float(ns)
    if ns >= 1e6:
        return f"{ns / 1e6:.2f}ms"
    if ns >= 1e3:
        return f"{ns / 1e3:.1f}us"
    return f"{ns:.0f}ns"


def fmt(key, value):
    if key.endswith("_ns"):
        return fmt_ns(value)
    if key == "thr_ops_s":
        return f"{float(value) / 1e6:.2f}M"
    if key == "rss_peak_mb":
        return f"{float(value):.0f}MB"
    return value


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--reps", type=int, default=5)
    parser.add_argument("--cases", default="A,B")
    parser.add_argument("--quick", action="store_true", help="1 rep, no warm-up run")
    args = parser.parse_args()
    cases = args.cases.split(",")
    reps = 1 if args.quick else args.reps

    os.makedirs(BIN, exist_ok=True)
    build()

    rows = []
    checksums = {}
    matrix = [m for m in DEFAULT_MATRIX if m[0] in cases]
    for case, label, binary, extra_env in matrix:
        print(f"# {case} {label}: warm-up + {reps} reps", file=sys.stderr)
        if not args.quick:
            run_once(binary, extra_env)  # discarded warm-up run
        runs = [run_once(binary, extra_env) for _ in range(reps)]
        median = dict(runs[0])
        for key in runs[0]:
            try:
                median[key] = statistics.median(float(r[key]) for r in runs)
            except ValueError:
                pass  # non-numeric fields (lang, checksum) keep the first run's value
        thrs = [float(r["thr_ops_s"]) for r in runs]
        spread = (max(thrs) - min(thrs)) / statistics.median(thrs) * 100
        rows.append((case, label, median, spread))
        workload = (case, extra_env.get("BENCH_HEAVY_EVERY", ""))
        for r in runs:
            checksums.setdefault(workload, set()).add(r["checksum"])

    for workload, sums in checksums.items():
        if len(sums) > 1:
            sys.exit(f"CHECKSUM MISMATCH in {workload}: {sums} — implementations diverge")

    print("\n| case | impl | " + " | ".join(h for _, h in COLUMNS) + " | thr spread |")
    print("|" + "---|" * (len(COLUMNS) + 3))
    for case, label, median, spread in rows:
        cells = " | ".join(fmt(k, median[k]) for k, _ in COLUMNS)
        print(f"| {case} | {label} | {cells} | {spread:.0f}% |")
    print("\nchecksums per workload:", {w: sorted(s) for w, s in checksums.items()})
    for case, label, median, _ in rows:
        if "numgc" in median:
            print(f"  {case} {label}: numgc={median['numgc']} "
                  f"gc_pause_total_ms={median['gc_pause_total_ms']} gogc={median['gogc']}")


if __name__ == "__main__":
    main()
