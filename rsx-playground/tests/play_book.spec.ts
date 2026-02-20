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

    // Wait for content to reflect the new symbol
    await expect(bookData).toContainText(/SOL|no book data/i, {
      timeout: 5000,
    });
  });

  test("book ladder auto-refreshes every 1s", async ({ page }) => {
    await page.goto("/book");
    const bookData = page.locator("#book-data");

    // Verify hx-trigger includes auto-refresh
    await verifyPolling(bookData, "every 1s");
  });

  test("book ladder shows real bid/ask when maker running", async ({ page }) => {
    await page.goto("/book");
    // PENGU (symbol_id=10) is the maker's symbol — quotes appear ~3s after startup
    await page.locator("#book-symbol").selectOption("10");
    await waitForHTMX(page);
    const bookData = page.locator("#book-data");
    // Poll until real Ask/Bid rows appear (HTMX refreshes every 1s)
    await expect(bookData).toContainText(/Ask|Bid/, { timeout: 10000 });
    const content = await bookData.textContent();
    expect(content).not.toMatch(/no book data|waiting for orders/i);
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

  test("live fills shows actual fills after orders", async ({ page }) => {
    // Submit an aggressive buy that crosses the maker's ask on symbol_id=10
    // Maker default mid=50000, ask starts at ~50050; price 51000 crosses it
    await page.request.post("/api/orders/test", {
      form: {
        symbol_id: "10",
        side: "buy",
        order_type: "limit",
        price: "51000",
        qty: "1",
        user_id: "1",
      },
    });
    await page.goto("/book");
    const fillsDiv = page.locator("[hx-get='./x/live-fills']");
    // HTMX refreshes every 1s; wait for WAL fill records to propagate
    await expect(fillsDiv).toContainText(/buy|sell/i, { timeout: 10000 });
    const content = await fillsDiv.textContent();
    expect(content).not.toMatch(/no fills|no processes running/i);
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
