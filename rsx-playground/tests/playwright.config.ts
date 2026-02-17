import { defineConfig } from "@playwright/test";

// Domain shards — deterministic order: routing → htmx → process-control → trade-ui
// retries: 0 globally; play-shard.sh blocks re-runs when failure signature unchanged.
export default defineConfig({
  testDir: ".",
  timeout: 15_000,
  retries: 0,
  use: {
    baseURL: "http://localhost:49171",
    headless: true,
  },
  reporter: "list",
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
