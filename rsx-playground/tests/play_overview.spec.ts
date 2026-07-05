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

  test("has scenario selector radios", async ({ page }) => {
    await page.goto("/overview");
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
    await expect(page.getByRole("heading", { name: /WAL stream lag/ })).toBeVisible();
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

// Exercises the REAL button → hx-post → cluster path (the e2e the
// visible-only assertions above skip). Clicking drives the actual
// build/start and stop of the exchange processes, so it needs a
// live harness; skipped when the dashboard is unreachable. Leaves
// the cluster running (+ maker) so downstream shards find it up.
test.describe("Start/Stop All buttons drive the cluster", () => {
  test("click Build & Start All → 6/6, Stop All → 0", async ({
    page,
    request,
  }) => {
    test.setTimeout(240_000);
    const health = await request.get("/healthz").catch(() => null);
    test.skip(
      !health || !health.ok(),
      "dashboard/cluster harness not reachable",
    );

    const running = async (): Promise<number> => {
      const r = await page.request.get("/healthz");
      if (!r.ok()) return -1;
      const j = await r.json();
      return j.processes_running;
    };
    const waitRunning = async (
      target: number,
      ms: number,
    ): Promise<number> => {
      const deadline = Date.now() + ms;
      let last = -1;
      while (Date.now() < deadline) {
        last = await running();
        if (last === target) return last;
        await new Promise((res) => setTimeout(res, 1500));
      }
      return last;
    };

    await page.goto("/overview");

    // Build & Start All → cluster comes up 6/6.
    await page
      .locator("button", { hasText: "Build & Start All" })
      .click();
    expect(await waitRunning(6, 180_000)).toBe(6);

    // Stop All → cluster drops to 0.
    await page.locator("button", { hasText: "Stop All" }).click();
    expect(await waitRunning(0, 60_000)).toBe(0);

    // Restore so the rest of the suite / the user find it running.
    await page
      .locator("button", { hasText: "Build & Start All" })
      .click();
    expect(await waitRunning(6, 180_000)).toBe(6);
    await request
      .post("/api/maker/start?confirm=yes", {
        headers: { "x-confirm": "yes" },
      })
      .catch(() => {});
  });
});
