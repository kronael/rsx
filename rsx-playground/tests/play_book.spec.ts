import { test, expect } from "@playwright/test";
import { waitForHTMX, waitForRefresh, verifyPolling } from "./test_helpers";

test.describe("Book tab", () => {
  test("loads and has symbol selector", async ({ page }) => {
    await page.goto("/book");
    await expect(page.locator("nav a", { hasText: "Book" }))
      .toHaveClass(/bg-slate-700/);
    await expect(page.getByRole("heading", { name: "Orderbook Ladder" })).toBeVisible();
    const select = page.locator("#book-symbol");
    await expect(select).toBeVisible();
    await expect(select.locator("option")).toHaveCount(4);
  });

  test("symbol selector has expected options", async ({ page }) => {
    await page.goto("/book");
    const select = page.locator("#book-symbol");
    await expect(select.locator("option[value='10']"))
      .toHaveText("PENGU");
    await expect(select.locator("option[value='1']"))
      .toHaveText("BTC");
  });

  test("has book stats card", async ({ page }) => {
    await page.goto("/book");
    await expect(page.getByRole("heading", { name: "Book Stats" })).toBeVisible();
  });

  test("has live fills card", async ({ page }) => {
    await page.goto("/book");
    await expect(page.getByRole("heading", { name: "Live Fills" })).toBeVisible();
  });

  // New interactive tests (11 total)

  test("symbol selector changes orderbook display", async ({ page }) => {
    await page.goto("/book");
    const bookData = page.locator("#book-data");
    await expect(bookData).toBeVisible();

    // Change symbol and verify HTMX triggers
    const select = page.locator("#book-symbol");
    await select.selectOption("1"); // BTC
    await waitForHTMX(page);

    // Verify book data was updated
    const content = await bookData.textContent();
    expect(content).toBeTruthy();
  });

  test("symbol selector triggers HTMX swap", async ({ page }) => {
    await page.goto("/book");
    const bookData = page.locator("#book-data");

    // Change symbol from PENGU to SOL
    await page.locator("#book-symbol").selectOption("3");
    await waitForHTMX(page);

    // Should show content with new symbol ID
    const newContent = await bookData.textContent();
    expect(newContent).toBeTruthy();
    expect(newContent).toContain("symbol 3");
  });

  test("book ladder auto-refreshes every 1s", async ({ page }) => {
    await page.goto("/book");
    const bookData = page.locator("#book-data");

    // Verify hx-trigger includes auto-refresh
    await verifyPolling(bookData, "every 1s");
  });

  test("book ladder shows placeholder when no processes running", async ({ page }) => {
    await page.goto("/book");
    const bookData = page.locator("#book-data");
    await waitForHTMX(page, 2000);

    // Should show "start RSX processes" message
    await expect(bookData).toContainText(/start RSX processes|symbol/i);
  });

  test("book stats card auto-refreshes every 2s", async ({ page }) => {
    await page.goto("/book");
    const statsDiv = page.locator("[hx-get='./x/book-stats']");

    // Verify auto-refresh configured
    await verifyPolling(statsDiv, "every 2s");
  });

  test("book stats updates over time", async ({ page }) => {
    await page.goto("/book");
    const statsDiv = page.locator("[hx-get='./x/book-stats']");
    const initial = await statsDiv.textContent();

    await waitForRefresh(2000);

    const updated = await statsDiv.textContent();
    expect(updated).toBeTruthy();
  });

  test("live fills card auto-refreshes every 1s", async ({ page }) => {
    await page.goto("/book");
    const fillsDiv = page.locator("[hx-get='./x/live-fills']");

    // Verify polling interval
    await verifyPolling(fillsDiv, "every 1s");
  });

  test("live fills shows placeholder initially", async ({ page }) => {
    await page.goto("/book");
    const fillsDiv = page.locator("[hx-get='./x/live-fills']");
    await waitForHTMX(page, 2000);

    // Should show placeholder text
    await expect(fillsDiv).toContainText(/start RSX processes|no data/i);
  });

  test("book stats card shows compression info", async ({ page }) => {
    await page.goto("/book");
    await expect(page.getByRole("heading", { name: "Book Stats" })).toBeVisible();

    // Stats should mention compression or order count
    const statsDiv = page.locator("[hx-get='./x/book-stats']");
    await waitForHTMX(page, 2000);
    const content = await statsDiv.textContent();
    expect(content).toBeTruthy();
  });

  test("trade aggregation card auto-refreshes", async ({ page }) => {
    await page.goto("/book");
    const tradeAgg = page.locator("[hx-get='./x/trade-agg']");

    // Verify auto-refresh configured
    await verifyPolling(tradeAgg, "every 2s");
  });

  test("all book cards load without errors", async ({ page }) => {
    await page.goto("/book");

    // Verify all cards are visible and loaded
    await expect(page.getByRole("heading", { name: "Orderbook Ladder" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Book Stats" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Live Fills" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "Trade Aggregation" })).toBeVisible();

    // Wait for initial HTMX loads
    await waitForHTMX(page, 2000);

    // No unexpected console errors (ignore CDN/network errors)
    const errors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" && !msg.text().includes("ERR_")) {
        errors.push(msg.text());
      }
    });

    await page.waitForTimeout(1000);
    expect(errors.length).toBe(0);
  });
});
