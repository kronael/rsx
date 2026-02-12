import { test, expect } from "@playwright/test";

test.describe("Orders tab", () => {
  test("loads with order form", async ({ page }) => {
    await page.goto("/orders");
    await expect(page.locator("nav a", { hasText: "Orders" }))
      .toHaveClass(/bg-slate-700/);
    await expect(page.getByRole("heading", { name: "Submit Order" })).toBeVisible();
    await expect(page.locator("select[name='symbol_id']"))
      .toBeVisible();
    await expect(page.locator("select[name='side']"))
      .toBeVisible();
    await expect(page.locator("input[name='price']"))
      .toBeVisible();
    await expect(page.locator("input[name='qty']"))
      .toBeVisible();
  });

  test("has order lifecycle trace card", async ({ page }) => {
    await page.goto("/orders");
    await expect(page.getByRole("heading", { name: "Order Lifecycle Trace" })).toBeVisible();
    await expect(page.locator("#trace-oid")).toBeVisible();
  });

  test("has recent orders card", async ({ page }) => {
    await page.goto("/orders");
    await expect(page.getByRole("heading", { name: "Recent Orders" })).toBeVisible();
  });

  test("has batch and stress test buttons", async ({ page }) => {
    await page.goto("/orders");
    await expect(page.locator("button", { hasText: "Batch (10)" }))
      .toBeVisible();
    await expect(page.locator("button", { hasText: "Stress (100)" }))
      .toBeVisible();
  });
});
