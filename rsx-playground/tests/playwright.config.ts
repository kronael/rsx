import { defineConfig } from "@playwright/test";
import path from "path";

// Domain shards — deterministic order:
//   routing → htmx → verify → readiness → process-control → market-maker → trade-ui
// retries: 0 globally; play-shard.sh blocks re-runs when failure signature unchanged.
// Reporter is set per-run via CLI --reporter flag (json + junit from play-shard.sh).
// PLAYWRIGHT_JUNIT_OUTPUT_NAME env var controls junit artifact path.
const shard = process.env.PW_SHARD ?? "unknown";
const artifactDir = path.join(
  __dirname, "..", "tmp", "play-artifacts", shard
);

export default defineConfig({
  globalSetup: "./global-setup.ts",
  testDir: ".",
  timeout: 15_000,
  retries: 0,
  // One worker at a time: prevents concurrent session-allocate races and
  // ensures the session-preflight project runs before product shards.
  workers: 1,
  use: {
    baseURL: "http://localhost:49171",
    headless: true,
  },
  // Reporter resolved from CLI --reporter when invoked via play-shard.sh.
  // Default "list" for direct npx playwright test invocations.
  reporter: process.env.PW_SHARD
    ? [
        ["json", { outputFile: path.join(artifactDir, "report.json") }],
        ["junit", { outputFile: path.join(artifactDir, "report.xml") }],
        ["list"],
      ]
    : [["list"]],
  webServer: {
    command: "bash -c 'source ../.venv/bin/activate && python ../server.py'",
    url: "http://localhost:49171",
    reuseExistingServer: true,
    timeout: 10000,
  },
  projects: [
    // Shard 0a: session-preflight — collision self-test (3 tests).
    // Runs first; all product shards depend on it via infra-smoke.
    {
      name: "session-preflight",
      testMatch: ["play_session.spec.ts"],
    },
    // Shard 0b: infra-smoke — session lifecycle smoke test (5 tests).
    // Verifies create/reuse/teardown guards are live after global-setup.
    // All product shards declare this as a dependency so they are skipped
    // immediately when session infra is broken (fail fast).
    {
      name: "infra-smoke",
      testMatch: ["play_infra.spec.ts"],
      dependencies: ["orch-smoke"],
    },
    // Shard 0c: orch-smoke — session collision lifecycle (4 tests).
    // Mutates session state: release → re-allocate → collision → restore.
    // Depends on session-preflight (collision guard verified first).
    // Must run before product shards so the restored session is valid.
    {
      name: "orch-smoke",
      testMatch: ["play_orch.spec.ts"],
      dependencies: ["session-preflight"],
    },
    // Shard 1: routing — navigation + high-level page loading (12 tests)
    {
      name: "routing",
      testMatch: [
        "play_navigation.spec.ts",
        "play_overview.spec.ts",
        "play_topology.spec.ts",
      ],
      dependencies: ["infra-smoke"],
    },
    // Shard 2: htmx-partials — HTMX data pages (49 tests)
    {
      name: "htmx-partials",
      testMatch: [
        "play_book.spec.ts",
        "play_risk.spec.ts",
        "play_wal.spec.ts",
        "play_logs.spec.ts",
        "play_faults.spec.ts",
      ],
      dependencies: ["infra-smoke"],
    },
    // Shard 2a-safety: safety, crash & handover (25 tests)
    {
      name: "safety",
      testMatch: ["play_safety.spec.ts"],
      dependencies: ["infra-smoke"],
    },
    // Shard 2b: verify — phase verification, single-worker lane (13 tests)
    // Separated from htmx-partials so orchestration invariants run after
    // all HTMX data pages are stable; workers: 1 is inherited globally but
    // the explicit dependency chain enforces sequential phase order.
    {
      name: "verify",
      testMatch: ["play_verify.spec.ts"],
      dependencies: ["htmx-partials"],
    },
    // Shard 2c: readiness — deterministic single-worker validation pipeline
    // Verifies core processes, maker, and book are ready before product
    // shards that exercise the live exchange path.  Declared after verify
    // so the phase chain is: infra-smoke → readiness → (process-control,
    // market-maker, trade-ui).
    {
      name: "readiness",
      testMatch: ["play_readiness.spec.ts"],
      dependencies: ["infra-smoke"],
    },
    // Shard 3: process-control — control + orders + stress + down-contract
    {
      name: "process-control",
      testMatch: [
        "play_control.spec.ts",
        "play_orders.spec.ts",
        "play_stress.spec.ts",
        "play_down_contract.spec.ts",
      ],
      dependencies: ["readiness"],
    },
    // Shard 4: market-maker — maker e2e (6 tests)
    {
      name: "market-maker",
      testMatch: ["play_maker.spec.ts"],
      use: { baseURL: "http://localhost:49171" },
      timeout: 60_000,
      dependencies: ["readiness"],
    },
    // Shard 5: trade-ui — React SPA (67 tests)
    // Depends on market-maker so the book is seeded with quotes before
    // live-orderbook tests run (explicit dependency, not positional).
    {
      name: "trade-ui",
      testMatch: [
        "play_trade.spec.ts",
      ],
      dependencies: ["market-maker"],
    },
    // Shard 6: latency — performance and memory bounds (15 tests)
    {
      name: "latency",
      testMatch: ["play_latency.spec.ts"],
      timeout: 120_000,
      dependencies: ["infra-smoke"],
    },
  ],
});
