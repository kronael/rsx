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
});
