import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests",
  testMatch: "*.spec.ts",
  timeout: 20_000,
  retries: 0,
  use: {
    baseURL: "http://localhost:4173",
    headless: true,
    viewport: { width: 1440, height: 900 },
    // Store screenshots/diffs next to tests
    screenshot: "only-on-failure",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  reporter: "list",
  // Start `vite preview` (serves dist/) before tests.
  // Run `bun run build` first, or set CI=true to skip.
  webServer: {
    command: "bun run preview -- --port 4173",
    url: "http://localhost:4173",
    reuseExistingServer: !process.env.CI,
    timeout: 30_000,
  },
});
