import { test, expect } from "@playwright/test";

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
});
