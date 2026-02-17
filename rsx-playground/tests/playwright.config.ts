import { defineConfig } from "@playwright/test";
import path from "path";

// Domain shards — deterministic order: routing → htmx → process-control → trade-ui
// retries: 0 globally; play-shard.sh blocks re-runs when failure signature unchanged.
// Reporter is set per-run via CLI --reporter flag (json + junit from play-shard.sh).
// PLAYWRIGHT_JUNIT_OUTPUT_NAME env var controls junit artifact path.
const shard = process.env.PW_SHARD ?? "unknown";
const artifactDir = path.join(
  __dirname, "..", "tmp", "play-artifacts", shard
);

export default defineConfig({
  testDir: ".",
  timeout: 15_000,
  retries: 0,
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
    // Shard 1: routing — navigation + high-level page loading (12 tests)
    {
      name: "routing",
      testMatch: [
        "play_navigation.spec.ts",
        "play_overview.spec.ts",
        "play_topology.spec.ts",
      ],
    },
    // Shard 2: htmx-partials — HTMX data pages (62 tests)
    {
      name: "htmx-partials",
      testMatch: [
        "play_book.spec.ts",
        "play_risk.spec.ts",
        "play_wal.spec.ts",
        "play_logs.spec.ts",
        "play_faults.spec.ts",
        "play_verify.spec.ts",
      ],
    },
    // Shard 3: process-control — control + orders (35 tests)
    {
      name: "process-control",
      testMatch: [
        "play_control.spec.ts",
        "play_orders.spec.ts",
      ],
    },
    // Shard 4: trade-ui — React SPA (67 tests)
    {
      name: "trade-ui",
      testMatch: [
        "play_trade.spec.ts",
      ],
    },
  ],
});
