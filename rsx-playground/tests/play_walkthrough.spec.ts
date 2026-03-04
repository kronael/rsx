import { test, expect } from "@playwright/test";

test.describe("Walkthrough", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/walkthrough");
  });

  // ── Hero ──

  test("page loads with hero title", async ({ page }) => {
    await expect(
      page.locator("h1"),
    ).toContainText("RSX Exchange");
  });

  // ── Launch panel ──

  test("launch panel is visible", async ({ page }) => {
    await expect(
      page.locator("#launch"),
    ).toBeVisible();
  });

  test("start all button exists", async ({ page }) => {
    await expect(
      page.locator("#wt-start-btn"),
    ).toBeVisible();
  });

  test("processes widget loads", async ({ page }) => {
    const proc = page.locator("#wt-processes");
    await expect(proc).toBeVisible();
    // HTMX loads on page load; wait for content
    await expect(proc).not.toContainText("Loading", {
      timeout: 10000,
    });
  });

  test("depth widget loads", async ({ page }) => {
    const depth = page.locator("#wt-depth");
    await expect(depth).toBeVisible();
  });

  // ── Live depth (with maker running) ──

  test(
    "depth shows bid and ask rows",
    async ({ page, request }) => {
      await request.post("/api/maker/start");
      // Poll book API until data present
      for (let i = 0; i < 10; i++) {
        const r = await request.get("/api/book/10");
        const b = await r.json();
        if (b.bids?.length >= 1 && b.asks?.length >= 1)
          break;
        await new Promise((r) =>
          setTimeout(r, 1000),
        );
      }

      await page.goto("/walkthrough");
      const depth = page.locator("#wt-depth");

      const bidRow = depth.locator(
        "[data-testid='bid-row']",
      );
      await expect(bidRow.first()).toBeVisible({
        timeout: 10000,
      });

      const askRow = depth.locator(
        "[data-testid='ask-row']",
      );
      await expect(askRow.first()).toBeVisible();

      expect(
        await bidRow.count(),
      ).toBeGreaterThanOrEqual(1);
      expect(
        await askRow.count(),
      ).toBeGreaterThanOrEqual(1);
    },
  );

  test(
    "depth bid price > 0",
    async ({ page, request }) => {
      await request.post("/api/maker/start");
      await page.goto("/walkthrough");

      const bidRow = page
        .locator("#wt-depth [data-testid='bid-row']")
        .first();
      await expect(bidRow).toBeVisible({
        timeout: 10000,
      });

      const px = await bidRow.getAttribute("data-px");
      expect(parseInt(px!, 10)).toBeGreaterThan(0);
    },
  );

  test(
    "depth ask price > 0",
    async ({ page, request }) => {
      await request.post("/api/maker/start");
      await page.goto("/walkthrough");

      const askRow = page
        .locator("#wt-depth [data-testid='ask-row']")
        .first();
      await expect(askRow).toBeVisible({
        timeout: 10000,
      });

      const px = await askRow.getAttribute("data-px");
      expect(parseInt(px!, 10)).toBeGreaterThan(0);
    },
  );

  test(
    "depth shows spread",
    async ({ page, request }) => {
      await request.post("/api/maker/start");
      await page.goto("/walkthrough");

      const spread = page.locator(
        "#wt-depth td:has-text('spread')",
      );
      await expect(spread).toBeVisible({
        timeout: 10000,
      });
    },
  );

  // ── Educational sections ──

  test("all 9 section anchors exist", async ({
    page,
  }) => {
    const ids = [
      "big-picture",
      "order-lifecycle",
      "matching-engine",
      "risk-engine",
      "wal-transport",
      "market-data",
      "mark-price",
      "benchmarks",
      "try-it",
    ];
    for (const id of ids) {
      await expect(
        page.locator(`#${id}`),
      ).toBeAttached();
    }
  });

  test("sections have expandable details", async ({
    page,
  }) => {
    const details = page.locator("details");
    const count = await details.count();
    expect(count).toBeGreaterThanOrEqual(9);
  });

  test("nav has section links", async ({ page }) => {
    const nav = page.locator("nav a");
    const count = await nav.count();
    expect(count).toBeGreaterThanOrEqual(9);
  });
});
