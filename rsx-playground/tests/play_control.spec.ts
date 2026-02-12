import { test, expect } from "@playwright/test";

test.describe("Control tab", () => {
  test("loads and has process control card", async ({ page }) => {
    await page.goto("/control");
    await expect(page.locator("nav a", { hasText: "Control" }))
      .toHaveClass(/bg-slate-700/);
    await expect(page.getByRole("heading", { name: "Process Control" })).toBeVisible();
  });

  test("has notes card with scenario commands", async ({ page }) => {
    await page.goto("/control");
    await expect(page.getByRole("heading", { name: "Notes" })).toBeVisible();
    await expect(page.locator("code").first()).toBeVisible();
  });

  test("has resource usage card", async ({ page }) => {
    await page.goto("/control");
    await expect(page.getByRole("heading", { name: "Resource Usage" })).toBeVisible();
  });
});
