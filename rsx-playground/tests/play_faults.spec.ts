import { test, expect } from "@playwright/test";

test.describe("Faults tab", () => {
  test("loads with fault injection card", async ({ page }) => {
    await page.goto("/faults");
    await expect(page.locator("nav a", { hasText: "Faults" }))
      .toHaveClass(/bg-slate-700/);
    await expect(page.getByRole("heading", { name: "Fault Injection" })).toBeVisible();
  });

  test("has recovery notes card", async ({ page }) => {
    await page.goto("/faults");
    await expect(page.getByRole("heading", { name: "Recovery Notes" })).toBeVisible();
    await expect(page.locator("main")).toContainText(
      "observe recovery"
    );
  });
});
