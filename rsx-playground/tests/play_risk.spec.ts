import { test, expect } from "@playwright/test";

test.describe("Risk tab", () => {
  test("loads and has user lookup", async ({ page }) => {
    await page.goto("/risk");
    await expect(page.locator("nav a", { hasText: "Risk" }))
      .toHaveClass(/bg-slate-700/);
    await expect(page.getByRole("heading", { name: "User Actions" })).toBeVisible();
    await expect(page.locator("#risk-uid")).toBeVisible();
    await expect(page.locator("button", { hasText: "Lookup" }))
      .toBeVisible();
  });

  test("has freeze and unfreeze buttons", async ({ page }) => {
    await page.goto("/risk");
    await expect(page.locator("button", { hasText: /^Freeze$/ }))
      .toBeVisible();
    await expect(page.locator("button", { hasText: /^Unfreeze$/ }))
      .toBeVisible();
  });

  test("has position heatmap card", async ({ page }) => {
    await page.goto("/risk");
    await expect(page.getByRole("heading", { name: "Position Heatmap" })).toBeVisible();
  });

  test("has margin ladder card", async ({ page }) => {
    await page.goto("/risk");
    await expect(page.getByRole("heading", { name: "Margin Ladder" })).toBeVisible();
  });

  test("has liquidation queue card", async ({ page }) => {
    await page.goto("/risk");
    await expect(page.getByRole("heading", { name: "Liquidation Queue" })).toBeVisible();
  });
});
