import { test, expect } from "@playwright/test";

test.describe("Logs tab", () => {
  test("loads and has filters", async ({ page }) => {
    await page.goto("/logs");
    await expect(page.locator("nav a", { hasText: "Logs" }))
      .toHaveClass(/bg-slate-700/);
    await expect(page.getByRole("heading", { name: "Unified Log" })).toBeVisible();
    await expect(page.locator("#log-process")).toBeVisible();
    await expect(page.locator("#log-level")).toBeVisible();
    await expect(page.locator("#log-search")).toBeVisible();
  });

  test("process filter has expected options", async ({ page }) => {
    await page.goto("/logs");
    const select = page.locator("#log-process");
    await expect(select.locator("option")).toHaveCount(7);
    await expect(select.locator("option[value='gateway']"))
      .toHaveText("gateway");
  });

  test("level filter has expected options", async ({ page }) => {
    await page.goto("/logs");
    const select = page.locator("#log-level");
    await expect(select.locator("option[value='error']"))
      .toHaveText("error");
  });

  test("has error aggregation card", async ({ page }) => {
    await page.goto("/logs");
    await expect(page.getByRole("heading", { name: "Error Aggregation" })).toBeVisible();
  });
});
