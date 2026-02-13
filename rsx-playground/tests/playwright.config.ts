import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: ".",
  testMatch: "play_*.spec.ts",
  timeout: 15_000,
  retries: 0,
  use: {
    baseURL: "http://localhost:49171",
    headless: true,
  },
  reporter: "list",
  webServer: {
    command: "source ../.venv/bin/activate && python ../server.py",
    url: "http://localhost:49171",
    reuseExistingServer: true,
    timeout: 10000,
  },
});
