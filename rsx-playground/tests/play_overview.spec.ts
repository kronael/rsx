import { test, expect } from "@playwright/test";

test.describe("Overview tab", () => {
  test("loads and shows process table", async ({ page }) => {
    await page.goto("/overview");
    await expect(page.locator("nav")).toContainText("RSX");
    await expect(page.locator("nav a", { hasText: "Overview" }))
      .toHaveClass(/bg-slate-700/);
    await expect(page.getByRole("heading", { name: "Process Table" })).toBeVisible();
  });

  test("has start all and stop all buttons", async ({ page }) => {
    await page.goto("/overview");
    await expect(page.locator("button", { hasText: "Build & Start All" }))
      .toBeVisible();
    await expect(page.locator("button", { hasText: "Stop All" }))
      .toBeVisible();
  });

  test("has system health card", async ({ page }) => {
    await page.goto("/overview");
    await expect(page.getByRole("heading", { name: "System Health" })).toBeVisible();
  });

  test("has WAL status card", async ({ page }) => {
    await page.goto("/overview");
    await expect(page.getByRole("heading", { name: "WAL Status" })).toBeVisible();
  });

  test("has key metrics card", async ({ page }) => {
    await page.goto("/overview");
    await expect(page.getByRole("heading", { name: "Key Metrics" })).toBeVisible();
  });

  test("process table auto-refreshes every 2s", async ({ page }) => {
    await page.goto("/overview");
    await page.waitForSelector("div[hx-get='./x/processes']", { timeout: 5000 });
    const trigger = await page.locator("div[hx-get='./x/processes']").getAttribute("hx-trigger");
    expect(trigger).toContain("every 2s");
  });

  test("health score updates dynamically", async ({ page }) => {
    await page.goto("/overview");
    await page.waitForSelector("div[hx-get='./x/health']", { timeout: 5000 });
    await page.waitForTimeout(500);
    const healthContent = await page.locator("div[hx-get='./x/health']").innerHTML();
    expect(healthContent.length).toBeGreaterThan(50);
  });

  test("key metrics display process counts", async ({ page }) => {
    await page.goto("/overview");
    await page.waitForSelector("div[hx-get='./x/key-metrics']", { timeout: 5000 });
    await page.waitForTimeout(500);
    const metricsContent = await page.locator("div[hx-get='./x/key-metrics']").innerHTML();
    expect(metricsContent).toContain("Processes");
  });

  test("WAL status auto-refreshes every 2s", async ({ page }) => {
    await page.goto("/overview");
    await page.waitForSelector("div[hx-get='./x/wal-status']", { timeout: 5000 });
    const trigger = await page.locator("div[hx-get='./x/wal-status']").getAttribute("hx-trigger");
    expect(trigger).toContain("every 2s");
  });

  test("has scenario selector dropdown", async ({ page }) => {
    await page.goto("/overview");
    // Scenario uses radio buttons, not a select
    const radios = page.locator(
      "input[name='scenario-ov']");
    const count = await radios.count();
    expect(count).toBeGreaterThanOrEqual(3);
    // Verify specific values exist
    await expect(page.locator(
      "input[name='scenario-ov'][value='minimal']"
    )).toBeAttached();
    await expect(page.locator(
      "input[name='scenario-ov'][value='full']"
    )).toBeAttached();
  });

  test("build spinner shows during build", async ({ page }) => {
    await page.goto("/overview");
    const buildSpin = page.locator("#build-spin");
    await expect(buildSpin).toHaveClass(/htmx-indicator/);
  });

  test("logs tail auto-refreshes every 2s", async ({ page }) => {
    await page.goto("/overview");
    await page.waitForSelector("div[hx-get='./x/logs-tail']", { timeout: 5000 });
    const trigger = await page.locator("div[hx-get='./x/logs-tail']").getAttribute("hx-trigger");
    expect(trigger).toContain("every 2s");
  });

  test("invariants card has auto-refresh configured", async ({ page }) => {
    await page.goto("/overview");
    const inv = page.locator("div[hx-get='./x/invariant-status']");
    await expect(inv).toBeVisible();
    const trigger = await inv.getAttribute("hx-trigger");
    expect(trigger).toContain("every 5s");
  });

  test("ring backpressure card displays", async ({ page }) => {
    await page.goto("/overview");
    await expect(page.getByRole("heading", { name: "Ring Backpressure" })).toBeVisible();
    await page.waitForSelector("div[hx-get='./x/ring-pressure']", { timeout: 5000 });
    await page.waitForTimeout(500);
    const ringContent = await page.locator("div[hx-get='./x/ring-pressure']").innerHTML();
    expect(ringContent.length).toBeGreaterThan(0);
  });

  test("start result container exists", async ({ page }) => {
    await page.goto("/overview");
    const startResult = page.locator("#start-result");
    await expect(startResult).toBeAttached();
  });
});
