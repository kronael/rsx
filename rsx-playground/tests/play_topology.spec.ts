import { test, expect } from "@playwright/test";

test.describe("Topology tab", () => {
  test("loads and shows process graph", async ({ page }) => {
    await page.goto("/topology");
    await expect(page.locator("nav a", { hasText: "Topology" }))
      .toHaveClass(/bg-slate-700/);
    await expect(page.getByRole("heading", { name: "Process Graph" })).toBeVisible();
    await expect(page.locator("pre")).toContainText("Gateway");
    await expect(page.locator("pre")).toContainText("Risk");
    await expect(page.locator("pre")).toContainText("Marketdata");
  });

  test("has core affinity card", async ({ page }) => {
    await page.goto("/topology");
    await expect(page.getByRole("heading", { name: "Core Affinity Map" })).toBeVisible();
  });

  test("has CMP connections card", async ({ page }) => {
    await page.goto("/topology");
    await expect(page.getByRole("heading", { name: "CMP Connections" })).toBeVisible();
  });

  test("has process list card", async ({ page }) => {
    await page.goto("/topology");
    await expect(page.getByRole("heading", { name: "Process List" })).toBeVisible();
  });
});
