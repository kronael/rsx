import { test, expect } from "@playwright/test";

test.describe("Verify tab", () => {
  test("loads with invariants card", async ({ page }) => {
    await page.goto("/verify");
    await expect(page.locator("nav a", { hasText: "Verify" }))
      .toHaveClass(/bg-slate-700/);
    await expect(page.getByRole("heading", { name: "Invariants" })).toBeVisible();
  });

  test("has Run All Checks button", async ({ page }) => {
    await page.goto("/verify");
    await expect(
      page.locator("button", { hasText: "Run All Checks" })
    ).toBeVisible();
  });

  test("has reconciliation card", async ({ page }) => {
    await page.goto("/verify");
    await expect(page.getByRole("heading", { name: "Reconciliation" })).toBeVisible();
  });

  test("has latency regression card", async ({ page }) => {
    await page.goto("/verify");
    await expect(page.getByRole("heading", { name: "Latency Regression" })).toBeVisible();
  });
});
